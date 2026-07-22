use crate::ui::i18n;
use crate::ui::shell::*;
use anyhow::Result;
use gpui::{App, Context, Window};
use miaominal_secrets::ProtectedPassphrase;
use miaominal_services::{LocalVaultMode, LocalVaultPassphraseChangeOutcome, LocalVaultTransition};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VaultUnlockSuccessStep {
    ApplyCredentialsTransition,
    FinishSettings,
    ResumeDeferredCommand,
}

fn vault_unlock_success_steps(has_deferred_command: bool) -> Vec<VaultUnlockSuccessStep> {
    let mut steps = vec![
        VaultUnlockSuccessStep::ApplyCredentialsTransition,
        VaultUnlockSuccessStep::FinishSettings,
    ];
    if has_deferred_command {
        steps.push(VaultUnlockSuccessStep::ResumeDeferredCommand);
    }
    steps
}

pub(in crate::ui::shell) trait LocalVaultRootExt: Sized {
    fn local_vault_operation_in_progress(&self, cx: &App) -> bool;

    fn run_local_vault_unlock_follow_up(
        &mut self,
        follow_up: DeferredAppCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    fn schedule_local_vault_unlock_follow_up(
        &mut self,
        follow_up: DeferredAppCommand,
        cx: &mut Context<Self>,
    );

    fn local_vault_primary_action_label(&self, cx: &App) -> String;

    fn local_vault_disable_action_label(&self, cx: &App) -> String;

    fn local_vault_change_action_label(&self, cx: &App) -> String;

    fn open_local_vault_passphrase_popup(
        &mut self,
        mode: LocalVaultPassphrasePopupMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    fn open_local_vault_passphrase_popup_in_active_window(
        &mut self,
        mode: LocalVaultPassphrasePopupMode,
        cx: &mut Context<Self>,
    );

    fn close_local_vault_passphrase_popup(&mut self, window: &mut Window, cx: &mut Context<Self>);

    fn handle_local_vault_action_request(
        &mut self,
        request: LocalVaultActionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    fn spawn_local_vault_unlock(&mut self, passphrase: ProtectedPassphrase, cx: &mut Context<Self>);

    fn apply_local_vault_operation_result(
        &mut self,
        result: LocalVaultOperationResult,
        cx: &mut Context<Self>,
    );

    fn finish_local_vault_operation(
        &mut self,
        result: LocalVaultOperationResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    fn finish_local_vault_operation_without_window(
        &mut self,
        result: LocalVaultOperationResult,
        cx: &mut Context<Self>,
    );

    fn finish_local_vault_unlock(
        &mut self,
        result: Result<LocalVaultUnlockResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    fn finish_local_vault_unlock_without_window(
        &mut self,
        result: Result<LocalVaultUnlockResult>,
        cx: &mut Context<Self>,
    );

    fn spawn_local_vault_disable(&mut self, cx: &mut Context<Self>);

    fn spawn_local_vault_enable(&mut self, passphrase: ProtectedPassphrase, cx: &mut Context<Self>);

    fn finish_local_vault_enable(
        &mut self,
        result: Result<LocalVaultEnableResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    fn finish_local_vault_enable_without_window(
        &mut self,
        result: Result<LocalVaultEnableResult>,
        cx: &mut Context<Self>,
    );

    fn lock_local_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<()>;

    fn spawn_local_vault_change_passphrase(
        &mut self,
        current_passphrase: ProtectedPassphrase,
        new_passphrase: ProtectedPassphrase,
        cx: &mut Context<Self>,
    );

    fn finish_local_vault_change_passphrase(
        &mut self,
        result: Result<LocalVaultChangePassphraseResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    fn finish_local_vault_change_passphrase_without_window(
        &mut self,
        result: Result<LocalVaultChangePassphraseResult>,
        cx: &mut Context<Self>,
    );

    fn finish_local_vault_disable(
        &mut self,
        result: Result<LocalVaultTransition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    fn finish_local_vault_disable_without_window(
        &mut self,
        result: Result<LocalVaultTransition>,
        cx: &mut Context<Self>,
    );

    fn disable_local_vault(
        &mut self,
        transition: LocalVaultTransition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()>;

    fn local_vault_secret_ids(&self, cx: &App) -> (Vec<String>, Vec<String>, Vec<String>);

    fn apply_local_vault_transition(
        &mut self,
        transition: LocalVaultTransition,
        cx: &mut Context<Self>,
    );

    fn finish_local_vault_auto_lock(&mut self, window: &mut Window, cx: &mut Context<Self>);

    fn finish_local_vault_auto_lock_without_window(&mut self, cx: &mut Context<Self>);
}

impl LocalVaultRootExt for AppView {
    fn local_vault_operation_in_progress(&self, cx: &App) -> bool {
        self.controllers
            .settings
            .read(cx)
            .local_vault_operation_in_progress()
    }
    fn run_local_vault_unlock_follow_up(
        &mut self,
        follow_up: DeferredAppCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match follow_up {
            DeferredAppCommand::Session(command) => match command {
                SessionDeferredCommand::OpenProfile(profile) => {
                    self.open_session_tab(*profile, window, cx);
                }
                SessionDeferredCommand::SaveProfile => {
                    let controller = self.controllers.session.clone();
                    controller.update(cx, |controller, cx| {
                        controller.continue_save_profile_after_unlock(window, cx);
                    });
                }
                SessionDeferredCommand::SavePortForwardRule => {
                    let controller = self.controllers.session.clone();
                    controller.update(cx, |controller, cx| {
                        controller.continue_save_port_forward_rule_after_unlock(window, cx);
                    });
                }
                SessionDeferredCommand::SaveSnippet => {
                    let controller = self.controllers.session.clone();
                    controller.update(cx, |controller, cx| {
                        controller.continue_save_snippet_after_unlock(window, cx);
                    });
                }
            },
            DeferredAppCommand::Keychain(command) => {
                let controller = self.controllers.keychain.clone();
                controller.update(cx, |controller, cx| {
                    controller.resume_deferred(command, window, cx);
                });
            }
            DeferredAppCommand::Settings(command) => match command {
                SettingsDeferredCommand::ResumeSync => {
                    self.controllers.settings.update(cx, |controller, cx| {
                        controller.trigger_sync_now(window, cx);
                    });
                }
                SettingsDeferredCommand::SaveSyncPassphrase(passphrase) => {
                    self.controllers.settings.update(cx, |controller, cx| {
                        controller.continue_save_sync_passphrase_after_unlock(passphrase, cx);
                    });
                }
                SettingsDeferredCommand::OpenSyncProviderConfig(provider) => {
                    self.open_sync_provider_config_popup(provider, window, cx);
                }
                SettingsDeferredCommand::SaveSyncProviderConfig(draft) => {
                    self.controllers.settings.update(cx, |controller, cx| {
                        controller.continue_save_sync_provider_config_after_unlock(draft, cx);
                    });
                }
                SettingsDeferredCommand::OpenAiProvider(provider_id) => {
                    self.edit_ai_provider(provider_id, window, cx);
                }
                SettingsDeferredCommand::SaveAiProvider(draft) => {
                    self.controllers.settings.update(cx, |controller, cx| {
                        controller.continue_save_ai_provider_after_unlock(draft, cx);
                    });
                }
                SettingsDeferredCommand::OpenWebSearchConfig => {
                    self.open_web_search_config_popup(window, cx);
                }
                SettingsDeferredCommand::SaveWebSearch(draft) => {
                    self.controllers.settings.update(cx, |controller, cx| {
                        controller.continue_save_web_search_after_unlock(draft, cx);
                    });
                }
                SettingsDeferredCommand::ClearSyncPassphrase => {
                    self.controllers.settings.update(cx, |controller, cx| {
                        controller.continue_clear_sync_passphrase_after_unlock(cx);
                    });
                }
                SettingsDeferredCommand::RevealSecret(target) => {
                    if target == SecretRevealTarget::HostPassword {
                        self.controllers.session.update(cx, |controller, cx| {
                            controller.reveal_host_password_input(window, cx);
                        });
                    } else {
                        self.controllers.settings.update(cx, |controller, cx| {
                            controller.continue_reveal_secret_after_unlock(target, window, cx);
                        });
                    }
                }
            },
            DeferredAppCommand::Agent(AgentDeferredCommand::ResumeRequest) => {
                self.sync_session_port_snapshot(cx);
                self.controllers.agent.update(cx, |controller, cx| {
                    controller.submit_session_agent_prompt(window, cx);
                });
            }
            DeferredAppCommand::Sftp(SftpDeferredCommand::OpenProfile { profile_id, owner }) => {
                if let Some(owner) = owner {
                    self.ensure_session_side_panel_sftp_tab(owner, cx);
                } else if let Some(profile) = self
                    .controllers
                    .session
                    .read(cx)
                    .query_port()
                    .profile(&profile_id)
                {
                    self.open_sftp_tab(profile, window, cx);
                } else {
                    self.shell.status_message = i18n::string("trusted.messages.profile_not_found");
                    cx.notify();
                }
            }
        }
    }
    fn schedule_local_vault_unlock_follow_up(
        &mut self,
        follow_up: DeferredAppCommand,
        cx: &mut Context<Self>,
    ) {
        let notification_window = cx.active_window();

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(0))
                .await;

            let Some(window_handle) = notification_window else {
                return;
            };

            let this_for_window = this.clone();
            let update_result = window_handle.update(cx, move |_, window, cx| {
                if let Err(error) = this_for_window.update(cx, move |this, cx| {
                    this.run_local_vault_unlock_follow_up(follow_up, window, cx);
                }) {
                    log::debug!("failed to run local vault follow-up action in window: {error:?}");
                }
            });

            if let Err(error) = update_result {
                log::debug!("failed to access active window for local vault follow-up: {error:?}");
            }
        })
        .detach();
    }
    fn local_vault_primary_action_label(&self, cx: &App) -> String {
        self.controllers
            .settings
            .read(cx)
            .local_vault_primary_action_label()
    }
    fn local_vault_disable_action_label(&self, cx: &App) -> String {
        self.controllers
            .settings
            .read(cx)
            .local_vault_disable_action_label()
    }
    fn local_vault_change_action_label(&self, cx: &App) -> String {
        self.controllers
            .settings
            .read(cx)
            .local_vault_change_action_label()
    }
    fn open_local_vault_passphrase_popup(
        &mut self,
        mode: LocalVaultPassphrasePopupMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_operation_in_progress(cx) {
            return;
        }

        let stable_key = DialogOverlaySnapshot::LocalVaultPassphrasePopup(mode).stable_key();
        self.shell
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        let controller = self.controllers.settings.clone();
        controller.update(cx, |controller, cx| {
            controller.open_local_vault_passphrase_popup(mode, window, cx);
        });
    }
    fn open_local_vault_passphrase_popup_in_active_window(
        &mut self,
        mode: LocalVaultPassphrasePopupMode,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_operation_in_progress(cx) {
            return;
        }

        let stable_key = DialogOverlaySnapshot::LocalVaultPassphrasePopup(mode).stable_key();
        self.shell
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        let controller = self.controllers.settings.clone();
        controller.update(cx, |controller, cx| {
            controller.open_local_vault_passphrase_popup_in_active_window(mode, cx);
        });
    }
    fn close_local_vault_passphrase_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let controller = self.controllers.settings.clone();
        if controller.update(cx, |controller, cx| {
            controller.close_local_vault_passphrase_popup(window, cx)
        }) {
            self.shell.deferred_app_command = None;
        }
    }
    fn handle_local_vault_action_request(
        &mut self,
        request: LocalVaultActionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_operation_in_progress(cx) {
            return;
        }

        match request {
            LocalVaultActionRequest::Enable { passphrase } => {
                self.spawn_local_vault_enable(passphrase, cx)
            }
            LocalVaultActionRequest::Unlock { passphrase } => {
                self.spawn_local_vault_unlock(passphrase, cx)
            }
            LocalVaultActionRequest::Lock => {
                if let Err(error) = self.lock_local_vault(window, cx) {
                    let action = self.local_vault_primary_action_label(cx);
                    let settings = self.controllers.settings.clone();
                    self.shell.status_message = settings.update(cx, |controller, cx| {
                        controller.finish_local_vault_error(&action, &error, window, cx)
                    });
                    cx.notify();
                }
            }
            LocalVaultActionRequest::Disable => self.spawn_local_vault_disable(cx),
            LocalVaultActionRequest::ChangePassphrase {
                current_passphrase,
                new_passphrase,
            } => self.spawn_local_vault_change_passphrase(current_passphrase, new_passphrase, cx),
        }
    }
    fn spawn_local_vault_unlock(
        &mut self,
        passphrase: ProtectedPassphrase,
        cx: &mut Context<Self>,
    ) {
        self.controllers.settings.update(cx, |controller, cx| {
            controller.start_vault_unlock(passphrase, cx)
        });
    }

    fn apply_local_vault_operation_result(
        &mut self,
        result: LocalVaultOperationResult,
        cx: &mut Context<Self>,
    ) {
        let notification_window = cx.active_window();

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(0))
                .await;

            if let Some(window_handle) = notification_window {
                let result = std::rc::Rc::new(std::cell::RefCell::new(Some(result)));
                let result_for_window = result.clone();
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    let Some(result) = result_for_window.borrow_mut().take() else {
                        return;
                    };
                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_local_vault_operation(result, window, cx);
                    }) {
                        log::debug!("failed to apply local vault operation result: {error:?}");
                    }
                });

                if let Err(error) = update_result {
                    log::debug!(
                        "failed to access active window for local vault operation: {error:?}"
                    );
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_local_vault_operation_without_window(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply local vault operation result without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_local_vault_operation_without_window(result, cx);
            }) {
                log::debug!(
                    "failed to apply local vault operation result without active window: {error:?}"
                );
            }
        })
        .detach();
    }

    fn finish_local_vault_operation(
        &mut self,
        result: LocalVaultOperationResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            LocalVaultOperationResult::Unlock(result) => {
                self.finish_local_vault_unlock(result, window, cx)
            }
            LocalVaultOperationResult::Enable(result) => {
                self.finish_local_vault_enable(result, window, cx)
            }
            LocalVaultOperationResult::Disable(result) => {
                self.finish_local_vault_disable(result, window, cx)
            }
            LocalVaultOperationResult::ChangePassphrase(result) => {
                self.finish_local_vault_change_passphrase(result, window, cx)
            }
            LocalVaultOperationResult::AutoLock => self.finish_local_vault_auto_lock(window, cx),
        }
    }

    fn finish_local_vault_operation_without_window(
        &mut self,
        result: LocalVaultOperationResult,
        cx: &mut Context<Self>,
    ) {
        match result {
            LocalVaultOperationResult::Unlock(result) => {
                self.finish_local_vault_unlock_without_window(result, cx)
            }
            LocalVaultOperationResult::Enable(result) => {
                self.finish_local_vault_enable_without_window(result, cx)
            }
            LocalVaultOperationResult::Disable(result) => {
                self.finish_local_vault_disable_without_window(result, cx)
            }
            LocalVaultOperationResult::ChangePassphrase(result) => {
                self.finish_local_vault_change_passphrase_without_window(result, cx)
            }
            LocalVaultOperationResult::AutoLock => {
                self.finish_local_vault_auto_lock_without_window(cx)
            }
        }
    }

    fn finish_local_vault_unlock(
        &mut self,
        result: Result<LocalVaultUnlockResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(unlock_result) => {
                let LocalVaultUnlockResult {
                    transition,
                    sync_secret_inputs,
                } = unlock_result;

                let has_follow_up = self.shell.deferred_app_command.is_some();
                let mut transition = Some(transition);
                let mut sync_secret_inputs = Some(sync_secret_inputs);
                let mut message = None;
                for step in vault_unlock_success_steps(has_follow_up) {
                    match step {
                        VaultUnlockSuccessStep::ApplyCredentialsTransition => {
                            self.apply_local_vault_transition(
                                transition.take().expect("vault transition is applied once"),
                                cx,
                            );
                        }
                        VaultUnlockSuccessStep::FinishSettings => {
                            let settings = self.controllers.settings.clone();
                            message = Some(settings.update(cx, |controller, cx| {
                                controller.finish_local_vault_unlock(
                                    sync_secret_inputs
                                        .take()
                                        .expect("vault sync inputs are applied once"),
                                    window,
                                    cx,
                                )
                            }));
                        }
                        VaultUnlockSuccessStep::ResumeDeferredCommand => {
                            if let Some(follow_up) = self.shell.deferred_app_command.take() {
                                self.schedule_local_vault_unlock_follow_up(follow_up, cx);
                            }
                        }
                    }
                }

                if !has_follow_up {
                    self.shell.status_message = message.expect("settings completion always runs");
                }
            }
            Err(error) => {
                let action = self.local_vault_primary_action_label(cx);
                let settings = self.controllers.settings.clone();
                self.shell.status_message = settings.update(cx, |controller, cx| {
                    controller.finish_local_vault_error(&action, &error, window, cx)
                });
            }
        }
        cx.notify();
    }
    fn finish_local_vault_unlock_without_window(
        &mut self,
        result: Result<LocalVaultUnlockResult>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(unlock_result) => {
                let LocalVaultUnlockResult {
                    transition,
                    sync_secret_inputs,
                } = unlock_result;

                self.apply_local_vault_transition(transition, cx);
                self.shell.deferred_app_command = None;
                let settings = self.controllers.settings.clone();
                self.shell.status_message = settings.update(cx, |controller, cx| {
                    controller.finish_local_vault_unlock_without_window(&sync_secret_inputs, cx)
                });
            }
            Err(error) => {
                let action = self.local_vault_primary_action_label(cx);
                let settings = self.controllers.settings.clone();
                self.shell.status_message = settings.update(cx, |controller, _| {
                    controller.finish_local_vault_error_without_window(&action, &error)
                });
            }
        }

        cx.notify();
    }
    fn spawn_local_vault_disable(&mut self, cx: &mut Context<Self>) {
        let (session_ids, managed_key_ids, ai_provider_ids) = self.local_vault_secret_ids(cx);
        self.controllers.settings.update(cx, |controller, cx| {
            controller.start_vault_disable(session_ids, managed_key_ids, ai_provider_ids, cx);
        });
    }
    fn spawn_local_vault_enable(
        &mut self,
        passphrase: ProtectedPassphrase,
        cx: &mut Context<Self>,
    ) {
        let (session_ids, managed_key_ids, ai_provider_ids) = self.local_vault_secret_ids(cx);
        self.controllers.settings.update(cx, |controller, cx| {
            controller.start_vault_enable(
                passphrase,
                session_ids,
                managed_key_ids,
                ai_provider_ids,
                cx,
            );
        });
    }
    fn finish_local_vault_enable(
        &mut self,
        result: Result<LocalVaultEnableResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(enable_result) => {
                let LocalVaultEnableResult {
                    passphrase,
                    vault_secrets,
                    vault_sync_engine,
                    sync_secret_inputs,
                    session_ids,
                    managed_key_ids,
                    ai_provider_ids,
                } = enable_result;

                let previous_secrets = self.controllers.settings.read(cx).secrets();
                let previous_sync_engine = self.controllers.settings.read(cx).sync_engine().clone();
                let settings = self.controllers.settings.clone();
                let apply_result = settings.update(cx, |controller, _| {
                    controller.apply_vault_enable(passphrase, vault_secrets, vault_sync_engine)
                });

                match apply_result {
                    Ok(transition) => {
                        self.apply_local_vault_transition(transition, cx);
                        self.shell.status_message = settings.update(cx, |controller, cx| {
                            controller.finish_local_vault_enable(sync_secret_inputs, window, cx)
                        });
                        SettingsController::delete_migrated_keyring_secrets(
                            &session_ids,
                            &managed_key_ids,
                            &ai_provider_ids,
                            &previous_secrets,
                            &previous_sync_engine,
                        );
                    }
                    Err(error) => {
                        let action = self.local_vault_primary_action_label(cx);
                        self.shell.status_message = settings.update(cx, |controller, cx| {
                            controller.finish_local_vault_error(&action, &error, window, cx)
                        });
                    }
                }
            }
            Err(error) => {
                let action = self.local_vault_primary_action_label(cx);
                let settings = self.controllers.settings.clone();
                self.shell.status_message = settings.update(cx, |controller, cx| {
                    controller.finish_local_vault_error(&action, &error, window, cx)
                });
            }
        }
        cx.notify();
    }
    fn finish_local_vault_enable_without_window(
        &mut self,
        result: Result<LocalVaultEnableResult>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(enable_result) => {
                let LocalVaultEnableResult {
                    passphrase,
                    vault_secrets,
                    vault_sync_engine,
                    sync_secret_inputs,
                    session_ids,
                    managed_key_ids,
                    ai_provider_ids,
                } = enable_result;

                let previous_secrets = self.controllers.settings.read(cx).secrets();
                let previous_sync_engine = self.controllers.settings.read(cx).sync_engine().clone();
                let settings = self.controllers.settings.clone();
                let apply_result = settings.update(cx, |controller, _| {
                    controller.apply_vault_enable(passphrase, vault_secrets, vault_sync_engine)
                });

                match apply_result {
                    Ok(transition) => {
                        self.apply_local_vault_transition(transition, cx);
                        self.shell.status_message = settings.update(cx, |controller, cx| {
                            controller
                                .finish_local_vault_enable_without_window(&sync_secret_inputs, cx)
                        });
                        SettingsController::delete_migrated_keyring_secrets(
                            &session_ids,
                            &managed_key_ids,
                            &ai_provider_ids,
                            &previous_secrets,
                            &previous_sync_engine,
                        );
                    }
                    Err(error) => {
                        let action = self.local_vault_primary_action_label(cx);
                        self.shell.status_message = settings.update(cx, |controller, _| {
                            controller.finish_local_vault_error_without_window(&action, &error)
                        });
                    }
                }
            }
            Err(error) => {
                let action = self.local_vault_primary_action_label(cx);
                let settings = self.controllers.settings.clone();
                self.shell.status_message = settings.update(cx, |controller, _| {
                    controller.finish_local_vault_error_without_window(&action, &error)
                });
            }
        }

        cx.notify();
    }
    fn lock_local_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        self.controllers.session.update(cx, |controller, cx| {
            controller.prepare_host_password_for_lock(window, cx);
        });
        let transition = self
            .controllers
            .settings
            .read(cx)
            .local_vault_lock_transition();
        self.apply_local_vault_transition(transition, cx);
        self.controllers.session.update(cx, |controller, cx| {
            controller.set_host_password_visibility(false, false, window, cx);
        });
        let settings = self.controllers.settings.clone();
        self.shell.status_message = settings.update(cx, |controller, cx| {
            controller.finish_local_vault_lock(window, cx)
        });
        cx.notify();
        Ok(())
    }
    fn spawn_local_vault_change_passphrase(
        &mut self,
        current_passphrase: ProtectedPassphrase,
        new_passphrase: ProtectedPassphrase,
        cx: &mut Context<Self>,
    ) {
        self.controllers.settings.update(cx, |controller, cx| {
            controller.start_vault_change_passphrase(current_passphrase, new_passphrase, cx);
        });
    }
    fn finish_local_vault_change_passphrase(
        &mut self,
        result: Result<LocalVaultChangePassphraseResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(change_result) => {
                let LocalVaultChangePassphraseResult {
                    outcome,
                    sync_secret_inputs,
                } = change_result;

                match outcome {
                    LocalVaultPassphraseChangeOutcome::Reopened(transition) => {
                        self.apply_local_vault_transition(transition, cx);
                        let settings = self.controllers.settings.clone();
                        self.shell.status_message = settings.update(cx, |controller, cx| {
                            controller.finish_local_vault_change_passphrase(
                                sync_secret_inputs,
                                window,
                                cx,
                            )
                        });
                    }
                    LocalVaultPassphraseChangeOutcome::Locked { transition, error } => {
                        self.apply_local_vault_transition(transition, cx);
                        let action = self.local_vault_change_action_label(cx);
                        let settings = self.controllers.settings.clone();
                        self.shell.status_message = settings.update(cx, |controller, cx| {
                            controller.finish_local_vault_change_passphrase_locked(
                                sync_secret_inputs,
                                &action,
                                &error,
                                window,
                                cx,
                            )
                        });
                    }
                }
            }
            Err(error) => {
                let action = self.local_vault_change_action_label(cx);
                let settings = self.controllers.settings.clone();
                self.shell.status_message = settings.update(cx, |controller, cx| {
                    controller
                        .finish_local_vault_change_passphrase_error(&action, &error, window, cx)
                });
            }
        }
        cx.notify();
    }
    fn finish_local_vault_change_passphrase_without_window(
        &mut self,
        result: Result<LocalVaultChangePassphraseResult>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(change_result) => {
                let LocalVaultChangePassphraseResult {
                    outcome,
                    sync_secret_inputs,
                } = change_result;

                match outcome {
                    LocalVaultPassphraseChangeOutcome::Reopened(transition) => {
                        self.apply_local_vault_transition(transition, cx);
                        let settings = self.controllers.settings.clone();
                        self.shell.status_message = settings.update(cx, |controller, cx| {
                            controller.finish_local_vault_change_passphrase_without_window(
                                &sync_secret_inputs,
                                cx,
                            )
                        });
                    }
                    LocalVaultPassphraseChangeOutcome::Locked { transition, error } => {
                        self.apply_local_vault_transition(transition, cx);
                        let action = self.local_vault_change_action_label(cx);
                        let settings = self.controllers.settings.clone();
                        self.shell.status_message = settings.update(cx, |controller, _| {
                            controller.finish_local_vault_error_without_window(&action, &error)
                        });
                    }
                }
            }
            Err(error) => {
                let action = self.local_vault_change_action_label(cx);
                let settings = self.controllers.settings.clone();
                self.shell.status_message = settings.update(cx, |controller, _| {
                    controller
                        .finish_local_vault_change_passphrase_error_without_window(&action, &error)
                });
            }
        }

        cx.notify();
    }
    fn finish_local_vault_disable(
        &mut self,
        result: Result<LocalVaultTransition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(transition) => {
                if let Err(error) = self.disable_local_vault(transition, window, cx) {
                    let action = self.local_vault_disable_action_label(cx);
                    let settings = self.controllers.settings.clone();
                    self.shell.status_message = settings.update(cx, |controller, cx| {
                        controller.finish_local_vault_disable_error(&action, &error, window, cx)
                    });
                }
            }
            Err(error) => {
                let action = self.local_vault_disable_action_label(cx);
                let settings = self.controllers.settings.clone();
                self.shell.status_message = settings.update(cx, |controller, cx| {
                    controller.finish_local_vault_disable_error(&action, &error, window, cx)
                });
            }
        }
        cx.notify();
    }
    fn finish_local_vault_disable_without_window(
        &mut self,
        result: Result<LocalVaultTransition>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(transition) => {
                let settings = self.controllers.settings.clone();
                match settings.update(cx, |controller, _| controller.apply_vault_disable()) {
                    Ok(()) => {
                        self.apply_local_vault_transition(transition, cx);

                        if let Err(error) = SettingsController::erase_vault_file() {
                            log::warn!(
                                "failed to erase local vault file after disabling vault: {error:?}"
                            );
                        }

                        self.shell.status_message = settings.update(cx, |controller, _| {
                            controller.finish_local_vault_disable_without_window()
                        });
                    }
                    Err(error) => {
                        let action = self.local_vault_disable_action_label(cx);
                        self.shell.status_message = settings.update(cx, |controller, _| {
                            controller
                                .finish_local_vault_disable_error_without_window(&action, &error)
                        });
                    }
                }
            }
            Err(error) => {
                let action = self.local_vault_disable_action_label(cx);
                let settings = self.controllers.settings.clone();
                self.shell.status_message = settings.update(cx, |controller, _| {
                    controller.finish_local_vault_disable_error_without_window(&action, &error)
                });
            }
        }

        cx.notify();
    }
    fn disable_local_vault(
        &mut self,
        transition: LocalVaultTransition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let settings = self.controllers.settings.clone();
        settings.update(cx, |controller, _| controller.apply_vault_disable())?;
        self.apply_local_vault_transition(transition, cx);

        if let Err(error) = SettingsController::erase_vault_file() {
            log::warn!("failed to erase local vault file after disabling vault: {error:?}");
        }

        self.shell.status_message = settings.update(cx, |controller, cx| {
            controller.finish_local_vault_disable(window, cx)
        });
        cx.notify();
        Ok(())
    }
    fn local_vault_secret_ids(&self, cx: &App) -> (Vec<String>, Vec<String>, Vec<String>) {
        (
            self.controllers
                .session
                .read(cx)
                .profiles()
                .iter()
                .map(|session| session.id.clone())
                .collect(),
            self.controllers.keychain.read(cx).managed_key_ids(),
            self.controllers.settings.read(cx).ai_provider_ids(),
        )
    }
    fn apply_local_vault_transition(
        &mut self,
        transition: LocalVaultTransition,
        cx: &mut Context<Self>,
    ) {
        let LocalVaultTransition {
            mode,
            secrets,
            sync_engine,
            session_passphrase,
        } = transition;
        let local_vault_status = match mode {
            LocalVaultMode::Disabled => LocalVaultStatus::Disabled,
            LocalVaultMode::Locked => LocalVaultStatus::Locked,
            LocalVaultMode::Unlocked => LocalVaultStatus::Unlocked,
        };
        self.controllers.settings.update(cx, |controller, _| {
            controller.replace_sync_engine(sync_engine);
            controller.set_local_vault_status(local_vault_status);
        });
        self.controllers
            .broadcast_credentials_changed(secrets, local_vault_status, cx);
        let settings = self.controllers.settings.clone();
        settings.update(cx, |controller, cx| {
            controller.set_local_vault_session_passphrase(session_passphrase);
            controller.sync_local_vault_auto_lock_task(cx);
        });
    }
    fn finish_local_vault_auto_lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let should_lock = {
            let settings = self.controllers.settings.read(cx);
            settings.local_vault_status() == LocalVaultStatus::Unlocked
                && settings
                    .settings()
                    .local_vault_auto_lock_duration
                    .duration()
                    .is_some()
        };
        if !should_lock {
            return;
        }

        self.controllers.session.update(cx, |controller, cx| {
            controller.prepare_host_password_for_lock(window, cx);
        });
        let transition = self
            .controllers
            .settings
            .read(cx)
            .local_vault_lock_transition();
        self.apply_local_vault_transition(transition, cx);
        self.controllers.session.update(cx, |controller, cx| {
            controller.set_host_password_visibility(false, false, window, cx);
        });
        let settings = self.controllers.settings.clone();
        self.shell.status_message = settings.update(cx, |controller, cx| {
            controller.finish_local_vault_lock(window, cx)
        });
        cx.notify();
    }
    fn finish_local_vault_auto_lock_without_window(&mut self, cx: &mut Context<Self>) {
        let should_lock = {
            let settings = self.controllers.settings.read(cx);
            settings.local_vault_status() == LocalVaultStatus::Unlocked
                && settings
                    .settings()
                    .local_vault_auto_lock_duration
                    .duration()
                    .is_some()
        };
        if !should_lock {
            return;
        }

        let transition = self
            .controllers
            .settings
            .read(cx)
            .local_vault_lock_transition();
        self.apply_local_vault_transition(transition, cx);
        self.controllers.session.update(cx, |controller, cx| {
            controller.set_host_password_visible(false, cx);
        });
        let settings = self.controllers.settings.clone();
        self.shell.status_message = settings.update(cx, |controller, _| {
            controller.finish_local_vault_lock_without_window()
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_credentials_are_broadcast_before_deferred_command_resume() {
        assert_eq!(
            vault_unlock_success_steps(true),
            vec![
                VaultUnlockSuccessStep::ApplyCredentialsTransition,
                VaultUnlockSuccessStep::FinishSettings,
                VaultUnlockSuccessStep::ResumeDeferredCommand,
            ]
        );
        assert_eq!(
            vault_unlock_success_steps(false),
            vec![
                VaultUnlockSuccessStep::ApplyCredentialsTransition,
                VaultUnlockSuccessStep::FinishSettings,
            ]
        );
    }
}
