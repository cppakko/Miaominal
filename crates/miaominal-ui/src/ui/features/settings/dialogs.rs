use super::*;
use crate::ui::components::md3_switch;
use gpui_kit::component::Disableable;
use miaominal_core::proxy::{ProxyAuthMode, ProxyProtocol};

fn sync_defect_summary(report: &miaominal_sync::capability::CapabilityReport) -> String {
    use miaominal_sync::capability::{CapabilityReason as Reason, EtagKind};
    let key = match report.reason {
        Some(Reason::VersionUnavailable) => match report.etag_kind {
            EtagKind::Weak => "settings.sync.dialogs.unsafe_write_consent.defect_weak",
            EtagKind::Missing => "settings.sync.dialogs.unsafe_write_consent.defect_missing",
            _ => "settings.sync.dialogs.unsafe_write_consent.defect_invalid",
        },
        Some(Reason::ConditionalRead) => "settings.sync.dialogs.unsafe_write_consent.defect_read",
        _ => "settings.sync.dialogs.unsafe_write_consent.defect_write",
    };
    i18n::string(key)
}

impl SettingsController {
    pub(super) fn render_ssh_bridge_policy_downgrade_prompt(
        &self,
        entity: OverlayTarget<Self>,
        _presentation: &SettingsOverlayPresentation,
        prompt: &PendingSshBridgePolicyDowngradeState,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let supporting_text = i18n::string_args(
            "settings.openssh_integration.security.downgrade_confirm",
            &[("level", &bridge_security_level_label(prompt.level))],
        );
        let controller_cancel = entity.clone();
        let controller_confirm = entity.clone();
        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "ssh-bridge-policy-downgrade-cancel",
                    i18n::string("settings.openssh_integration.security.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_ssh_bridge_policy_downgrade(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "ssh-bridge-policy-downgrade-confirm",
                    i18n::string("settings.openssh_integration.security.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, _, cx| {
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_ssh_bridge_policy_downgrade(cx);
                    });
                }),
            );

        render_basic_dialog(
            "ssh-bridge-policy-downgrade",
            i18n::string("settings.openssh_integration.security_policy_group.title"),
            Some(supporting_text),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    pub(super) fn render_sync_direction_prompt(
        &self,
        entity: OverlayTarget<Self>,
        _presentation: &SettingsOverlayPresentation,
        _prompt: &PendingSyncDirectionState,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let controller_cancel = entity.clone();
        let controller_pull = entity.clone();
        let controller_push = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "sync-direction-cancel",
                    i18n::string("settings.sync.dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_sync_direction(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "sync-direction-pull",
                    i18n::string("settings.sync.dialogs.direction.pull"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_pull.update(cx, |controller, cx| {
                        controller.select_sync_pull(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "sync-direction-push",
                    i18n::string("settings.sync.dialogs.direction.push"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, window, cx| {
                    controller_push.update(cx, |controller, cx| {
                        controller.select_sync_push(window, cx);
                    });
                }),
            );

        render_basic_dialog(
            "sync-direction",
            i18n::string("settings.sync.dialogs.direction.title"),
            Some(i18n::string("settings.sync.dialogs.direction.message")),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    pub(super) fn render_sync_pull_confirm_prompt(
        &self,
        entity: OverlayTarget<Self>,
        _presentation: &SettingsOverlayPresentation,
        prompt: &PendingSyncPullConfirmState,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let controller_cancel = entity.clone();
        let controller_force_push = entity.clone();
        let controller_confirm = entity.clone();

        let (title_key, message_key) = match prompt.reason {
            SyncPullConfirmReason::Manual => (
                "settings.sync.dialogs.pull_confirm.title",
                "settings.sync.dialogs.pull_confirm.message",
            ),
            SyncPullConfirmReason::RemoteChanged => (
                "settings.sync.dialogs.pull_confirm.remote_changed_title",
                "settings.sync.dialogs.pull_confirm.remote_changed_message",
            ),
            SyncPullConfirmReason::BothSidesChanged => (
                "settings.sync.dialogs.pull_confirm.both_sides_changed_title",
                "settings.sync.dialogs.pull_confirm.both_sides_changed_message",
            ),
            SyncPullConfirmReason::UnsafeProviderWrite => (
                "settings.sync.dialogs.pull_confirm.unsafe_write_title",
                "settings.sync.dialogs.pull_confirm.unsafe_write_message",
            ),
            SyncPullConfirmReason::MissingSyncBaseline => (
                "settings.sync.dialogs.pull_confirm.missing_baseline_title",
                "settings.sync.dialogs.pull_confirm.missing_baseline_message",
            ),
        };

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "sync-pull-confirm-cancel",
                    i18n::string("settings.sync.dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_sync_pull_confirm(cx);
                    });
                }),
            )
            .when(prompt.reason != SyncPullConfirmReason::Manual, |this| {
                this.child(
                    basic_dialog_action_button(
                        "sync-pull-confirm-force-push",
                        i18n::string("settings.sync.dialogs.pull_confirm.force_push"),
                        BasicDialogActionTone::Default,
                    )
                    .on_click(move |_, window, cx| {
                        controller_force_push.update(cx, |controller, cx| {
                            controller.confirm_sync_force_push(window, cx);
                        });
                    }),
                )
            })
            .child(
                basic_dialog_action_button(
                    "sync-pull-confirm-accept",
                    i18n::string("settings.sync.dialogs.pull_confirm.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, window, cx| {
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_sync_pull(window, cx);
                    });
                }),
            );

        render_basic_dialog(
            "sync-pull-confirm",
            i18n::string(title_key),
            Some(i18n::string(message_key)),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    pub(super) fn render_sync_unsafe_write_consent_prompt(
        &self,
        entity: OverlayTarget<Self>,
        _presentation: &SettingsOverlayPresentation,
        prompt: &PendingSyncUnsafeWriteConsentState,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let controller_cancel = entity.clone();
        let controller_confirm = entity.clone();
        let roles = miaominal_settings::current_theme().material.roles;

        let body = v_flex()
            .w_full()
            .min_w(px(0.0))
            .gap_2()
            .child(i18n::string(
                "settings.sync.dialogs.unsafe_write_consent.message",
            ))
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .text_size(miaominal_settings::FontSize::Input.scaled())
                    .line_height(miaominal_settings::scaled_line_height(16.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string_args(
                        "settings.sync.dialogs.unsafe_write_consent.defect",
                        &[("defect", &sync_defect_summary(&prompt.capability))],
                    )),
            );

        render_basic_dialog(
            "sync-unsafe-write-consent",
            i18n::string("settings.sync.dialogs.unsafe_write_consent.title"),
            None,
            Some(body.into_any_element()),
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    basic_dialog_action_button(
                        "sync-unsafe-write-consent-cancel",
                        i18n::string("settings.sync.dialogs.unsafe_write_consent.cancel"),
                        BasicDialogActionTone::Default,
                    )
                    .on_click(move |_, _, cx| {
                        controller_cancel.update(cx, |controller, cx| {
                            controller.cancel_sync_unsafe_write_consent(cx);
                        });
                    }),
                )
                .child(
                    basic_dialog_action_button(
                        "sync-unsafe-write-consent-confirm",
                        i18n::string("settings.sync.dialogs.unsafe_write_consent.confirm"),
                        BasicDialogActionTone::Default,
                    )
                    .on_click(move |_, _, cx| {
                        controller_confirm.update(cx, |controller, cx| {
                            controller.confirm_sync_unsafe_write_consent(cx);
                        });
                    }),
                )
                .into_any_element(),
            exit_progress,
        )
    }

    pub(super) fn render_local_vault_disable_confirm_prompt(
        &self,
        entity: OverlayTarget<Self>,
        _presentation: &SettingsOverlayPresentation,
        _prompt: &PendingLocalVaultDisableConfirmState,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let controller_cancel = entity.clone();
        let controller_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "local-vault-disable-confirm-cancel",
                    i18n::string("settings.sync.dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_local_vault_disable_confirm(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "local-vault-disable-confirm-accept",
                    i18n::string("settings.sync.vault.dialogs.disable_confirm.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, _, cx| {
                    controller_confirm.update(cx, |controller, cx| {
                        controller.confirm_local_vault_disable(cx);
                    });
                }),
            );

        render_basic_dialog(
            "local-vault-disable-confirm",
            i18n::string("settings.sync.vault.dialogs.disable_confirm.title"),
            Some(i18n::string(
                "settings.sync.vault.dialogs.disable_confirm.message",
            )),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    pub(super) fn render_local_data_reset_confirm_prompt(
        &self,
        entity: OverlayTarget<Self>,
        _presentation: &SettingsOverlayPresentation,
        _prompt: &PendingLocalDataResetConfirmState,
        exit_progress: Option<f32>,
    ) -> gpui_kit::AnyElement {
        let controller_cancel = entity.clone();
        let controller_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "local-data-reset-confirm-cancel",
                    i18n::string("dialogs.common.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    controller_cancel.update(cx, |controller, cx| {
                        controller.cancel_local_data_reset_confirm(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "local-data-reset-confirm-continue",
                    i18n::string("settings.about.reset_local.confirm.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, window, cx| {
                    controller_confirm.update(cx, |controller, cx| {
                        controller.continue_local_data_reset_confirm(window, cx);
                    });
                }),
            );

        render_basic_dialog(
            "local-data-reset-confirm",
            i18n::string("settings.about.reset_local.confirm.title"),
            Some(i18n::string("settings.about.reset_local.confirm.message")),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    pub(super) fn render_local_vault_passphrase_popup(
        &self,
        entity: OverlayTarget<Self>,
        presentation: &SettingsOverlayPresentation,
        mode: LocalVaultPassphrasePopupMode,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        _cx: &App,
    ) -> gpui_kit::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = entity.clone();
        let settings_forms = presentation.forms.clone();
        let operation_in_progress = presentation.local_vault_unlock_in_progress();
        let current_input = settings_forms.local_vault_current_passphrase_input.clone();
        let input = settings_forms.local_vault_passphrase_input.clone();
        let confirmation_input = settings_forms
            .local_vault_passphrase_confirmation_input
            .clone();
        let local_vault_auto_lock_duration_select =
            settings_forms.local_vault_auto_lock_duration_select.clone();
        let entity_cancel = entity.clone();
        let controller_submit = settings_controller.clone();
        let title = presentation.local_vault_passphrase_popup_title(mode);
        let requires_passphrase_confirmation = mode
            == LocalVaultPassphrasePopupMode::ChangePassphrase
            || (mode == LocalVaultPassphrasePopupMode::PrimaryAction
                && presentation.local_vault_status() == LocalVaultStatus::Disabled);
        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_color(rgb(roles.primary))
                    .child(Icon::new(AppIcon::Vault).size(px(128.0))),
            )
            .when(
                mode == LocalVaultPassphrasePopupMode::ChangePassphrase,
                |this| {
                    this.child(surface_secret_text_input_stack(
                        i18n::string("settings.sync.vault.current_passphrase.label"),
                        current_input.clone(),
                        crate::ui::components::SecretTextInputStackOptions {
                            surface: TextInputSurface::Low,
                            size: gpui_kit::component::Size::Large,
                            required: true,
                            disabled: operation_in_progress,
                            trailing: None,
                            reveal_icon: presentation.secret_reveal_icon(
                                &SecretRevealTarget::LocalVaultCurrentPassphrase,
                            ),
                        },
                        {
                            let controller = settings_controller.clone();
                            move |window, cx| {
                                controller.update(cx, |controller, cx| {
                                    controller.toggle_secret_visibility(
                                        SecretRevealTarget::LocalVaultCurrentPassphrase,
                                        window,
                                        cx,
                                    );
                                });
                            }
                        },
                    ))
                },
            )
            .child(surface_secret_text_input_stack(
                if mode == LocalVaultPassphrasePopupMode::ChangePassphrase {
                    i18n::string("settings.sync.vault.new_passphrase.label")
                } else {
                    i18n::string("settings.sync.vault.passphrase.label")
                },
                input.clone(),
                crate::ui::components::SecretTextInputStackOptions {
                    surface: TextInputSurface::Low,
                    size: gpui_kit::component::Size::Large,
                    required: true,
                    disabled: operation_in_progress,
                    trailing: None,
                    reveal_icon: presentation
                        .secret_reveal_icon(&SecretRevealTarget::LocalVaultPassphrase),
                },
                {
                    let controller = settings_controller.clone();
                    move |window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.toggle_secret_visibility(
                                SecretRevealTarget::LocalVaultPassphrase,
                                window,
                                cx,
                            );
                        });
                    }
                },
            ))
            .when(requires_passphrase_confirmation, |this| {
                this.child(surface_secret_text_input_stack(
                    if mode == LocalVaultPassphrasePopupMode::ChangePassphrase {
                        i18n::string("settings.sync.vault.confirm_new_passphrase.label")
                    } else {
                        i18n::string("settings.sync.vault.confirm_passphrase.label")
                    },
                    confirmation_input.clone(),
                    crate::ui::components::SecretTextInputStackOptions {
                        surface: TextInputSurface::Low,
                        size: gpui_kit::component::Size::Large,
                        required: true,
                        disabled: operation_in_progress,
                        trailing: None,
                        reveal_icon: presentation.secret_reveal_icon(
                            &SecretRevealTarget::LocalVaultPassphraseConfirmation,
                        ),
                    },
                    {
                        let controller = settings_controller.clone();
                        move |window, cx| {
                            controller.update(cx, |controller, cx| {
                                controller.toggle_secret_visibility(
                                    SecretRevealTarget::LocalVaultPassphraseConfirmation,
                                    window,
                                    cx,
                                );
                            });
                        }
                    },
                ))
            })
            .when(
                mode == LocalVaultPassphrasePopupMode::PrimaryAction,
                |this| {
                    this.child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .child(field_label(
                                i18n::string("settings.sync.vault.auto_lock_duration.label"),
                                false,
                            ))
                            .child(
                                md3_select(&local_vault_auto_lock_duration_select)
                                    .large()
                                    .w_full()
                                    .rounded(px(14.0))
                                    .border_0()
                                    .bg(rgb(roles.surface_container_low))
                                    .disabled(operation_in_progress),
                            ),
                    )
                },
            )
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("local-vault-passphrase-popup-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, window, cx| {
                        entity_cancel.update(cx, |this, cx| {
                            this.close_local_vault_passphrase_popup(window, cx);
                        });
                    }),
            )
            .child(if operation_in_progress {
                div()
                    .id("local-vault-passphrase-popup-submit-spinner")
                    .min_w(px(116.0))
                    .min_h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::ui::components::md3_spinner(18.0))
                    .into_any_element()
            } else {
                Button::new("local-vault-passphrase-popup-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.primary))
                    .label(presentation.local_vault_passphrase_popup_title(mode))
                    .on_click(move |_, window, cx| {
                        controller_submit.update(cx, |controller, cx| {
                            controller.submit_local_vault_passphrase_popup_action(mode, window, cx);
                        });
                    })
                    .into_any_element()
            })
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                title,
                None,
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
            "local-vault-passphrase",
            exit_progress,
            move |window, cx| {
                entity.update(cx, |this, cx| {
                    this.close_local_vault_passphrase_popup(window, cx);
                });
            },
        )
    }

    pub(super) fn render_sync_passphrase_popup(
        &self,
        entity: OverlayTarget<Self>,
        presentation: &SettingsOverlayPresentation,
        _popup: PendingSyncPassphrasePopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        _cx: &App,
    ) -> gpui_kit::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = entity.clone();
        let settings_forms = presentation.forms.clone();
        let operation_in_progress = presentation.sync_passphrase_operation_in_progress();
        let save_in_progress = presentation.sync_passphrase_save_in_progress();
        let input = settings_forms.sync_passphrase_input.clone();
        let confirmation_input = settings_forms.sync_passphrase_confirmation_input.clone();
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();
        let title = presentation.sync_passphrase_action_label();
        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_color(rgb(roles.primary))
                    .child(Icon::new(AppIcon::Key).size(px(128.0))),
            )
            .child(surface_secret_text_input_stack(
                i18n::string("settings.sync.encryption.passphrase.label"),
                input.clone(),
                crate::ui::components::SecretTextInputStackOptions {
                    surface: TextInputSurface::Low,
                    size: gpui_kit::component::Size::Large,
                    required: true,
                    disabled: operation_in_progress,
                    trailing: None,
                    reveal_icon: presentation
                        .secret_reveal_icon(&SecretRevealTarget::SyncPassphrase),
                },
                {
                    let controller = settings_controller.clone();
                    move |window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.toggle_secret_visibility(
                                SecretRevealTarget::SyncPassphrase,
                                window,
                                cx,
                            );
                        });
                    }
                },
            ))
            .child(surface_secret_text_input_stack(
                i18n::string("settings.sync.encryption.passphrase.confirm_passphrase.label"),
                confirmation_input.clone(),
                crate::ui::components::SecretTextInputStackOptions {
                    surface: TextInputSurface::Low,
                    size: gpui_kit::component::Size::Large,
                    required: true,
                    disabled: operation_in_progress,
                    trailing: None,
                    reveal_icon: presentation
                        .secret_reveal_icon(&SecretRevealTarget::SyncPassphraseConfirmation),
                },
                {
                    let controller = settings_controller.clone();
                    move |window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.toggle_secret_visibility(
                                SecretRevealTarget::SyncPassphraseConfirmation,
                                window,
                                cx,
                            );
                        });
                    }
                },
            ))
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("sync-passphrase-popup-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, window, cx| {
                        controller_cancel.update(cx, |controller, cx| {
                            controller.close_sync_passphrase_popup(window, cx);
                        });
                    }),
            )
            .child(if save_in_progress {
                div()
                    .id("sync-passphrase-popup-submit-spinner")
                    .min_w(px(116.0))
                    .min_h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::ui::components::md3_spinner(18.0))
                    .into_any_element()
            } else {
                Button::new("sync-passphrase-popup-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.primary))
                    .label(title.clone())
                    .on_click(move |_, window, cx| {
                        controller_submit.update(cx, |controller, cx| {
                            controller.submit_sync_passphrase_popup_action(window, cx);
                        });
                    })
                    .into_any_element()
            })
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                title,
                None,
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
            "sync-passphrase",
            exit_progress,
            move |window, cx| {
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_sync_passphrase_popup(window, cx);
                });
            },
        )
    }

    pub(super) fn render_ai_provider_popup(
        &self,
        entity: OverlayTarget<Self>,
        presentation: &SettingsOverlayPresentation,
        _popup: PendingAiProviderPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        _cx: &App,
    ) -> gpui_kit::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = entity.clone();
        let settings_forms = presentation.forms.clone();
        let kind_select = settings_forms.ai_provider_kind_select.clone();
        let name_input = settings_forms.ai_provider_name_input.clone();
        let model_input = settings_forms.ai_provider_model_input.clone();
        let base_url_input = settings_forms.ai_provider_base_url_input.clone();
        let api_key_input = settings_forms.ai_provider_api_key_input.clone();
        let temperature_input = settings_forms.ai_provider_temperature_input.clone();
        let max_tokens_input = settings_forms.ai_provider_max_tokens_input.clone();
        let context_window_input = settings_forms.ai_provider_context_window_input.clone();
        let editing_provider_id = presentation.editing_ai_provider_id().map(str::to_owned);
        let provider_id = editing_provider_id.clone().unwrap_or_default();
        let save_in_progress = presentation.ai_provider_save_in_progress();
        let api_key_load_in_progress =
            presentation.ai_provider_api_key_load_in_progress_for(&provider_id);
        let operation_in_progress = save_in_progress || api_key_load_in_progress;
        let target = SecretRevealTarget::AiProviderApiKey(provider_id);
        let reveal_icon = presentation.secret_reveal_icon(&target);
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();

        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(field_label(
                        i18n::string("settings.ai_providers.kind.label"),
                        true,
                    ))
                    .child(
                        div().w_full().min_w(px(320.0)).child(
                            md3_select(&kind_select)
                                .large()
                                .w_full()
                                .bg(rgb(roles.surface_container_low)),
                        ),
                    ),
            )
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.name.label"),
                name_input,
                TextInputSurface::Low,
                true,
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.model.label"),
                model_input,
                TextInputSurface::Low,
                false,
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.base_url.label"),
                base_url_input,
                TextInputSurface::Low,
                false,
            ))
            .child(surface_secret_text_input_stack(
                i18n::string("settings.ai_providers.api_key.label"),
                api_key_input,
                crate::ui::components::SecretTextInputStackOptions {
                    surface: TextInputSurface::Low,
                    size: gpui_kit::component::Size::Large,
                    required: true,
                    disabled: operation_in_progress,
                    trailing: api_key_load_in_progress.then(|| {
                        div()
                            .id("ai-provider-api-key-load-spinner")
                            .size(px(30.0))
                            .rounded(px(10.0))
                            .bg(rgb(roles.surface_container_highest))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(crate::ui::components::md3_spinner(16.0))
                            .into_any_element()
                    }),
                    reveal_icon,
                },
                {
                    let controller = settings_controller.clone();
                    move |window, cx| {
                        let target = target.clone();
                        controller.update(cx, |controller, cx| {
                            controller.toggle_secret_visibility(target, window, cx);
                        });
                    }
                },
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.temperature.label"),
                temperature_input,
                TextInputSurface::Low,
                false,
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.max_tokens.label"),
                max_tokens_input,
                TextInputSurface::Low,
                false,
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.ai_providers.context_window.label"),
                context_window_input,
                TextInputSurface::Low,
                false,
            ))
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_between()
            .gap_3()
            .child(if let Some(provider_id) = editing_provider_id {
                let controller_delete = settings_controller.clone();
                Button::new("ai-provider-popup-delete")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.error))
                    .label(i18n::string("settings.ai_providers.actions.delete"))
                    .on_click(move |_, window, cx| {
                        let provider_id = provider_id.clone();
                        controller_delete.update(cx, |controller, cx| {
                            controller.delete_ai_provider(provider_id, window, cx);
                        });
                    })
                    .into_any_element()
            } else {
                div().into_any_element()
            })
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Button::new("ai-provider-popup-cancel")
                            .ghost()
                            .border_0()
                            .rounded(px(20.0))
                            .large()
                            .disabled(save_in_progress)
                            .text_color(rgb(roles.on_surface_variant))
                            .label(i18n::string("dialogs.common.cancel"))
                            .on_click(move |_, window, cx| {
                                controller_cancel.update(cx, |controller, cx| {
                                    controller.close_ai_provider_popup(window, cx);
                                });
                            }),
                    )
                    .child(if save_in_progress {
                        div()
                            .id("ai-provider-popup-submit-spinner")
                            .min_w(px(116.0))
                            .min_h(px(32.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(crate::ui::components::md3_spinner(18.0))
                            .into_any_element()
                    } else {
                        Button::new("ai-provider-popup-submit")
                            .ghost()
                            .border_0()
                            .rounded(px(20.0))
                            .large()
                            .disabled(operation_in_progress)
                            .text_color(rgb(roles.primary))
                            .label(i18n::string("settings.ai_providers.actions.save"))
                            .on_click(move |_, window, cx| {
                                controller_submit.update(cx, |controller, cx| {
                                    controller.submit_ai_provider_save(window, cx);
                                });
                            })
                            .into_any_element()
                    }),
            )
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                i18n::string("settings.ai_providers.editor_group.title"),
                Some(i18n::string(
                    "settings.ai_providers.editor_group.description",
                )),
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
            "ai-provider",
            exit_progress,
            move |window, cx| {
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_ai_provider_popup(window, cx);
                });
            },
        )
    }

    pub(super) fn render_web_search_config_popup(
        &self,
        entity: OverlayTarget<Self>,
        presentation: &SettingsOverlayPresentation,
        _popup: PendingWebSearchConfigPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        _cx: &App,
    ) -> gpui_kit::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = entity.clone();
        let settings_forms = presentation.forms.clone();
        let kind_select = settings_forms.web_search_kind_select.clone();
        let api_key_input = settings_forms.web_search_api_key_input.clone();
        let endpoint_input = settings_forms.web_search_endpoint_input.clone();
        let max_results_input = settings_forms.web_search_max_results_input.clone();
        let save_in_progress = presentation.web_search_save_in_progress();
        let current_kind = presentation.settings().web_search.kind;
        let api_key_required = current_kind.requires_api_key();
        let endpoint_required = current_kind == miaominal_settings::WebSearchProviderKind::SearXng;
        let target = SecretRevealTarget::WebSearchApiKey;
        let reveal_icon = presentation.secret_reveal_icon(&target);
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_toggle = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();

        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(field_label(
                        i18n::string("settings.web_search.kind.label"),
                        true,
                    ))
                    .child(
                        div().w_full().min_w(px(320.0)).child(
                            md3_select(&kind_select)
                                .large()
                                .w_full()
                                .bg(rgb(roles.surface_container_low)),
                        ),
                    ),
            )
            .child(surface_secret_text_input_stack(
                i18n::string("settings.web_search.api_key.label"),
                api_key_input,
                crate::ui::components::SecretTextInputStackOptions {
                    surface: TextInputSurface::Low,
                    size: gpui_kit::component::Size::Large,
                    required: api_key_required,
                    disabled: save_in_progress,
                    trailing: None,
                    reveal_icon,
                },
                move |window, cx| {
                    controller_toggle.update(cx, |controller, cx| {
                        controller.toggle_secret_visibility(
                            SecretRevealTarget::WebSearchApiKey,
                            window,
                            cx,
                        );
                    });
                },
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.web_search.endpoint.label"),
                endpoint_input,
                TextInputSurface::Low,
                endpoint_required,
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.web_search.max_results.label"),
                max_results_input,
                TextInputSurface::Low,
                true,
            ))
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("web-search-config-popup-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(save_in_progress)
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, window, cx| {
                        controller_cancel.update(cx, |controller, cx| {
                            controller.close_web_search_config_popup(window, cx);
                        });
                    }),
            )
            .child(if save_in_progress {
                div()
                    .id("web-search-config-popup-submit-spinner")
                    .min_w(px(116.0))
                    .min_h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::ui::components::md3_spinner(18.0))
                    .into_any_element()
            } else {
                Button::new("web-search-config-popup-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .text_color(rgb(roles.primary))
                    .label(i18n::string("settings.web_search.actions.save"))
                    .on_click(move |_, window, cx| {
                        controller_submit.update(cx, |controller, cx| {
                            controller.submit_web_search_settings_save(window, cx);
                        });
                    })
                    .into_any_element()
            })
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                i18n::string("settings.web_search.group.title"),
                Some(i18n::string("settings.web_search.group.description")),
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
            "web-search-config",
            exit_progress,
            move |window, cx| {
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_web_search_config_popup(window, cx);
                });
            },
        )
    }

    pub(super) fn render_sync_provider_config_popup(
        &self,
        entity: OverlayTarget<Self>,
        presentation: &SettingsOverlayPresentation,
        popup: PendingSyncProviderConfigPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        _cx: &App,
    ) -> gpui_kit::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = entity.clone();
        let settings_forms = presentation.forms.clone();
        let save_in_progress =
            presentation.sync_provider_config_save_in_progress_for(popup.provider);
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();

        let (title, description, popup_body, popup_key) = match popup.provider {
            SyncProvider::GithubGist => {
                let token_input = settings_forms.sync_github_token_input.clone();
                let gist_id_input = settings_forms.sync_github_gist_id_input.clone();
                let target = SecretRevealTarget::SyncGithubToken;
                let reveal_icon = presentation.secret_reveal_icon(&target);
                let controller_toggle = settings_controller.clone();
                (
                    i18n::string("settings.sync.gist_group.title"),
                    i18n::string("settings.sync.gist_group.description"),
                    v_flex()
                        .w_full()
                        .gap_5()
                        .child(surface_secret_text_input_stack(
                            i18n::string("settings.sync.gist.token.label"),
                            token_input,
                            crate::ui::components::SecretTextInputStackOptions {
                                surface: TextInputSurface::Low,
                                size: gpui_kit::component::Size::Large,
                                required: true,
                                disabled: save_in_progress,
                                trailing: None,
                                reveal_icon,
                            },
                            move |window, cx| {
                                controller_toggle.update(cx, |controller, cx| {
                                    controller.toggle_secret_visibility(
                                        SecretRevealTarget::SyncGithubToken,
                                        window,
                                        cx,
                                    );
                                });
                            },
                        ))
                        .child(surface_text_input_stack(
                            i18n::string("settings.sync.gist.gist_id.label"),
                            gist_id_input,
                            TextInputSurface::Low,
                            false,
                        ))
                        .into_any_element(),
                    "sync-provider-gist",
                )
            }
            SyncProvider::WebDav => {
                let url_input = settings_forms.sync_webdav_url_input.clone();
                let username_input = settings_forms.sync_webdav_username_input.clone();
                let password_input = settings_forms.sync_webdav_password_input.clone();
                let target = SecretRevealTarget::SyncWebdavPassword;
                let reveal_icon = presentation.secret_reveal_icon(&target);
                let controller_toggle = settings_controller.clone();
                (
                    i18n::string("settings.sync.webdav_group.title"),
                    i18n::string("settings.sync.webdav_group.description"),
                    v_flex()
                        .w_full()
                        .gap_5()
                        .child(surface_text_input_stack(
                            i18n::string("settings.sync.webdav.url.label"),
                            url_input,
                            TextInputSurface::Low,
                            true,
                        ))
                        .child(surface_text_input_stack(
                            i18n::string("settings.sync.webdav.username.label"),
                            username_input,
                            TextInputSurface::Low,
                            true,
                        ))
                        .child(surface_secret_text_input_stack(
                            i18n::string("settings.sync.webdav.password.label"),
                            password_input,
                            crate::ui::components::SecretTextInputStackOptions {
                                surface: TextInputSurface::Low,
                                size: gpui_kit::component::Size::Large,
                                required: true,
                                disabled: save_in_progress,
                                trailing: None,
                                reveal_icon,
                            },
                            move |window, cx| {
                                controller_toggle.update(cx, |controller, cx| {
                                    controller.toggle_secret_visibility(
                                        SecretRevealTarget::SyncWebdavPassword,
                                        window,
                                        cx,
                                    );
                                });
                            },
                        ))
                        .into_any_element(),
                    "sync-provider-webdav",
                )
            }
            SyncProvider::None => (
                i18n::string("settings.sync.provider_group.title"),
                i18n::string("settings.sync.provider_group.description"),
                div().into_any_element(),
                "sync-provider-none",
            ),
        };

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("sync-provider-config-popup-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(save_in_progress)
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, window, cx| {
                        controller_cancel.update(cx, |controller, cx| {
                            controller.close_sync_provider_config_popup(window, cx);
                        });
                    }),
            )
            .child(if save_in_progress {
                div()
                    .id("sync-provider-config-popup-submit-spinner")
                    .min_w(px(116.0))
                    .min_h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::ui::components::md3_spinner(18.0))
                    .into_any_element()
            } else {
                Button::new("sync-provider-config-popup-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .text_color(rgb(roles.primary))
                    .label(i18n::string("settings.sync.save_action"))
                    .on_click(move |_, _window, cx| {
                        controller_submit.update(cx, |controller, cx| {
                            controller.submit_sync_provider_config_popup_action(cx);
                        });
                    })
                    .into_any_element()
            })
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                title,
                Some(description),
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
            popup_key,
            exit_progress,
            move |window, cx| {
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_sync_provider_config_popup(window, cx);
                });
            },
        )
    }

    pub(super) fn render_proxy_config_popup(
        &self,
        entity: OverlayTarget<Self>,
        presentation: &SettingsOverlayPresentation,
        popup: PendingProxyConfigPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        cx: &App,
    ) -> gpui_kit::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = entity.clone();
        let settings_forms = presentation.forms.clone();
        let protocol = presentation.selected_proxy_protocol(cx);
        let auth_mode = presentation.selected_proxy_auth_mode(cx);
        let resolve_dns = presentation.proxy_resolve_dns_through_proxy();
        let password_clear_requested = presentation.proxy_password_clear_requested();
        let has_saved_password = presentation
            .proxies()
            .iter()
            .find(|proxy| proxy.id == popup.proxy_id)
            .is_some_and(|proxy| proxy.has_stored_password);
        let password_status = if password_clear_requested {
            i18n::string("settings.proxies.password.clear_pending")
        } else if has_saved_password {
            i18n::string("settings.proxies.password.keep_hint")
        } else {
            i18n::string("settings.proxies.password.new_hint")
        };
        let title = if popup.is_new {
            i18n::string("settings.proxies.editor.add_title")
        } else {
            i18n::string("settings.proxies.editor.edit_title")
        };

        let controller_dns = settings_controller.clone();
        let controller_clear_password = settings_controller.clone();
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();

        let mut popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(surface_text_input_stack(
                i18n::string("settings.proxies.fields.name"),
                settings_forms.proxy_name_input.clone(),
                TextInputSurface::Low,
                true,
            ))
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(field_label(
                        i18n::string("settings.proxies.fields.protocol"),
                        true,
                    ))
                    .child(
                        md3_select(&settings_forms.proxy_protocol_select)
                            .large()
                            .w_full()
                            .bg(rgb(roles.surface_container_low)),
                    ),
            )
            .child(surface_text_input_stack(
                i18n::string("settings.proxies.fields.host"),
                settings_forms.proxy_host_input.clone(),
                TextInputSurface::Low,
                true,
            ))
            .child(surface_text_input_stack(
                i18n::string("settings.proxies.fields.port"),
                settings_forms.proxy_port_input.clone(),
                TextInputSurface::Low,
                true,
            ))
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(field_label(
                        i18n::string("settings.proxies.fields.auth_mode"),
                        true,
                    ))
                    .child(
                        md3_select(&settings_forms.proxy_auth_mode_select)
                            .large()
                            .w_full()
                            .bg(rgb(roles.surface_container_low)),
                    ),
            );

        if protocol == ProxyProtocol::Socks5 {
            popup_body = popup_body.child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_color(rgb(roles.on_surface))
                                    .child(i18n::string("settings.proxies.fields.resolve_dns")),
                            )
                            .child(
                                div()
                                    .text_color(rgb(roles.on_surface_variant))
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .child(i18n::string(
                                        "settings.proxies.fields.resolve_dns_description",
                                    )),
                            ),
                    )
                    .child(
                        md3_switch("proxy-config-popup-resolve-dns")
                            .checked(resolve_dns)
                            .on_click(move |_, _, cx| {
                                controller_dns.update(cx, |controller, cx| {
                                    controller.toggle_proxy_resolve_dns(cx);
                                });
                            }),
                    ),
            );
        }

        if auth_mode == ProxyAuthMode::UsernamePassword {
            popup_body = popup_body
                .child(surface_text_input_stack(
                    i18n::string("settings.proxies.fields.username"),
                    settings_forms.proxy_username_input.clone(),
                    TextInputSurface::Low,
                    true,
                ))
                .child(surface_text_input_stack(
                    i18n::string("settings.proxies.fields.password"),
                    settings_forms.proxy_password_input.clone(),
                    TextInputSurface::Low,
                    !has_saved_password,
                ))
                .child(
                    div()
                        .text_color(rgb(roles.on_surface_variant))
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .child(password_status),
                );
            if has_saved_password && !password_clear_requested {
                popup_body = popup_body.child(
                    Button::new("proxy-config-popup-clear-password")
                        .ghost()
                        .border_0()
                        .rounded(px(20.0))
                        .large()
                        .text_color(rgb(roles.error))
                        .label(i18n::string("settings.proxies.actions.clear_password"))
                        .on_click(move |_, window, cx| {
                            controller_clear_password.update(cx, |controller, cx| {
                                controller.clear_proxy_password(window, cx);
                            });
                        }),
                );
            }
        }

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("proxy-config-popup-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, window, cx| {
                        controller_cancel.update(cx, |controller, cx| {
                            controller.close_proxy_config_popup(window, cx);
                        });
                    }),
            )
            .child(
                Button::new("proxy-config-popup-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .text_color(rgb(roles.primary))
                    .label(i18n::string("settings.proxies.actions.save"))
                    .on_click(move |_, window, cx| {
                        controller_submit.update(cx, |controller, cx| {
                            controller.save_proxy(window, cx);
                        });
                    }),
            )
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                title,
                Some(i18n::string("settings.proxies.group.description")),
                Some(popup_body.into_any_element()),
                actions,
                bottom_popup_viewport_height,
            ),
            "proxy-config",
            exit_progress,
            move |window, cx| {
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_proxy_config_popup(window, cx);
                });
            },
        )
    }

    pub(super) fn render_sync_passphrase_clear_confirm_popup(
        &self,
        entity: OverlayTarget<Self>,
        presentation: &SettingsOverlayPresentation,
        _popup: PendingSyncPassphraseClearConfirmPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        _cx: &App,
    ) -> gpui_kit::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = entity.clone();
        let operation_in_progress = presentation.sync_passphrase_operation_in_progress();
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();
        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_color(rgb(roles.error))
                    .child(Icon::new(AppIcon::Trash).size(px(128.0))),
            )
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .line_height(miaominal_settings::scaled_line_height(20.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string(
                        "settings.sync.encryption.passphrase.clear_confirm.message",
                    )),
            )
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("sync-passphrase-clear-confirm-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, _window, cx| {
                        controller_cancel.update(cx, |controller, cx| {
                            controller.close_sync_passphrase_clear_confirm_popup(cx);
                        });
                    }),
            )
            .child(
                Button::new("sync-passphrase-clear-confirm-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .disabled(operation_in_progress)
                    .text_color(rgb(roles.error))
                    .label(i18n::string(
                        "settings.sync.encryption.passphrase.clear_confirm.confirm",
                    ))
                    .on_click(move |_, _window, cx| {
                        controller_submit.update(cx, |controller, cx| {
                            controller.submit_sync_passphrase_clear_confirm_popup_action(cx);
                        });
                    }),
            )
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                i18n::string("settings.sync.encryption.passphrase.clear_confirm.title"),
                None,
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
            "sync-passphrase-clear-confirm",
            exit_progress,
            move |_window, cx| {
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_sync_passphrase_clear_confirm_popup(cx);
                });
            },
        )
    }

    pub(super) fn render_local_data_reset_confirmation_popup(
        &self,
        entity: OverlayTarget<Self>,
        presentation: &SettingsOverlayPresentation,
        _popup: PendingLocalDataResetConfirmationPopupState,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
        _cx: &App,
    ) -> gpui_kit::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let settings_controller = entity.clone();
        let input = presentation
            .forms
            .local_data_reset_confirmation_input
            .clone();
        let controller_cancel = settings_controller.clone();
        let controller_submit = settings_controller.clone();
        let controller_dismiss = settings_controller.clone();
        let popup_body = v_flex()
            .w_full()
            .gap_5()
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_color(rgb(roles.error))
                    .child(Icon::new(AppIcon::Trash).size(px(128.0))),
            )
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .line_height(miaominal_settings::scaled_line_height(20.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string("settings.about.reset_local.popup.description")),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(field_label(
                        i18n::string("settings.about.reset_local.popup.field_label"),
                        true,
                    ))
                    .child(
                        surface_text_input(&input, TextInputSurface::Low)
                            .large()
                            .text_color(rgb(roles.on_surface)),
                    ),
            )
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                Button::new("local-data-reset-popup-cancel")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .text_color(rgb(roles.on_surface_variant))
                    .label(i18n::string("dialogs.common.cancel"))
                    .on_click(move |_, window, cx| {
                        controller_cancel.update(cx, |controller, cx| {
                            controller.close_local_data_reset_confirmation_popup(window, cx);
                        });
                    }),
            )
            .child(
                Button::new("local-data-reset-popup-submit")
                    .ghost()
                    .border_0()
                    .rounded(px(20.0))
                    .large()
                    .text_color(rgb(roles.error))
                    .label(i18n::string("settings.about.reset_local.popup.confirm"))
                    .on_click(move |_, window, cx| {
                        controller_submit.update(cx, |controller, cx| {
                            controller
                                .submit_local_data_reset_confirmation_popup_action(window, cx);
                        });
                    }),
            )
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                i18n::string("settings.about.reset_local.popup.title"),
                None,
                Some(popup_body),
                actions,
                bottom_popup_viewport_height,
            ),
            "local-data-reset-confirmation",
            exit_progress,
            move |window, cx| {
                controller_dismiss.update(cx, |controller, cx| {
                    controller.close_local_data_reset_confirmation_popup(window, cx);
                });
            },
        )
    }
}
