use super::*;
use crate::ui::contracts::overlay::OverlaySnapshot;

#[derive(Clone)]
pub(super) enum SettingsOverlay {
    SyncDirection(PendingSyncDirectionState),
    SyncPullConfirm(PendingSyncPullConfirmState),
    SyncUnsafeWriteConsent(PendingSyncUnsafeWriteConsentState),
    LocalVaultDisableConfirm(PendingLocalVaultDisableConfirmState),
    LocalDataResetConfirm(PendingLocalDataResetConfirmState),
    SshBridgePolicyDowngrade(PendingSshBridgePolicyDowngradeState),
    LocalDataResetConfirmationPopup(PendingLocalDataResetConfirmationPopupState),
    SyncPassphraseClearConfirmPopup(PendingSyncPassphraseClearConfirmPopupState),
    SyncPassphrasePopup(PendingSyncPassphrasePopupState),
    AiProviderPopup(PendingAiProviderPopupState),
    WebSearchConfigPopup(PendingWebSearchConfigPopupState),
    SyncProviderConfigPopup(PendingSyncProviderConfigPopupState),
    ProxyConfigPopup(PendingProxyConfigPopupState),
    LocalVaultPassphrasePopup(LocalVaultPassphrasePopupMode),
}

impl SettingsOverlay {
    fn surface_key(&self) -> &'static str {
        match self {
            Self::SyncDirection(..) => "sync-direction",
            Self::SyncPullConfirm(..) => "sync-pull-confirm",
            Self::SyncUnsafeWriteConsent(..) => "sync-unsafe-write-consent",
            Self::LocalVaultDisableConfirm(..) => "local-vault-disable-confirm",
            Self::LocalDataResetConfirm(..) => "local-data-reset-confirm",
            Self::SshBridgePolicyDowngrade(..) => "ssh-bridge-policy-downgrade",
            Self::LocalDataResetConfirmationPopup(..) => "local-data-reset-confirmation",
            Self::SyncPassphraseClearConfirmPopup(..) => "sync-passphrase-clear-confirm",
            Self::SyncPassphrasePopup(..) => "sync-passphrase",
            Self::AiProviderPopup(..) => "ai-provider",
            Self::WebSearchConfigPopup(..) => "web-search-config",
            Self::SyncProviderConfigPopup(..) => "sync-provider-config",
            Self::ProxyConfigPopup(..) => "proxy-config",
            Self::LocalVaultPassphrasePopup(..) => "local-vault-passphrase",
        }
    }
}

impl SettingsController {
    pub(in crate::ui) fn overlay_snapshots(
        &self,
        entity: Entity<Self>,
        cx: &App,
    ) -> Vec<OverlaySnapshot> {
        let mut snapshots = Vec::new();
        if let Some(prompt) = self.sync_direction {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::SyncDirection(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.sync_pull_confirm {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::SyncPullConfirm(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.sync_unsafe_write_consent.clone() {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::SyncUnsafeWriteConsent(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.local_vault_disable_confirm {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::LocalVaultDisableConfirm(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.local_data_reset_confirm {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::LocalDataResetConfirm(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.pending_ssh_bridge_policy_downgrade {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::SshBridgePolicyDowngrade(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.local_data_reset_confirmation_popup {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::LocalDataResetConfirmationPopup(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.sync_passphrase_clear_confirm_popup {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::SyncPassphraseClearConfirmPopup(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.sync_passphrase_popup {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::SyncPassphrasePopup(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.ai_provider_popup {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::AiProviderPopup(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.web_search_config_popup {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::WebSearchConfigPopup(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.sync_provider_config_popup {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::SyncProviderConfigPopup(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.proxy_config_popup.clone() {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::ProxyConfigPopup(prompt),
                entity.clone(),
                cx,
            ));
        }
        if let Some(prompt) = self.local_vault_passphrase_popup {
            snapshots.push(self.overlay_snapshot(
                SettingsOverlay::LocalVaultPassphrasePopup(prompt),
                entity.clone(),
                cx,
            ));
        }
        snapshots
    }

    fn overlay_active(&self, prompt: &SettingsOverlay) -> bool {
        match prompt {
            SettingsOverlay::SyncDirection(..) => self.sync_direction.is_some(),
            SettingsOverlay::SyncPullConfirm(..) => self.sync_pull_confirm.is_some(),
            SettingsOverlay::SyncUnsafeWriteConsent(..) => self.sync_unsafe_write_consent.is_some(),
            SettingsOverlay::LocalVaultDisableConfirm(..) => {
                self.local_vault_disable_confirm.is_some()
            }
            SettingsOverlay::LocalDataResetConfirm(..) => self.local_data_reset_confirm.is_some(),
            SettingsOverlay::SshBridgePolicyDowngrade(..) => {
                self.pending_ssh_bridge_policy_downgrade.is_some()
            }
            SettingsOverlay::LocalDataResetConfirmationPopup(..) => {
                self.local_data_reset_confirmation_popup.is_some()
            }
            SettingsOverlay::SyncPassphraseClearConfirmPopup(..) => {
                self.sync_passphrase_clear_confirm_popup.is_some()
            }
            SettingsOverlay::SyncPassphrasePopup(..) => self.sync_passphrase_popup.is_some(),
            SettingsOverlay::AiProviderPopup(..) => self.ai_provider_popup.is_some(),
            SettingsOverlay::WebSearchConfigPopup(..) => self.web_search_config_popup.is_some(),
            SettingsOverlay::SyncProviderConfigPopup(..) => {
                self.sync_provider_config_popup.is_some()
            }
            SettingsOverlay::ProxyConfigPopup(..) => self.proxy_config_popup.is_some(),
            SettingsOverlay::LocalVaultPassphrasePopup(..) => {
                self.local_vault_passphrase_popup.is_some()
            }
        }
    }

    pub(super) fn overlay_snapshot(
        &self,
        prompt: SettingsOverlay,
        entity: Entity<Self>,
        cx: &App,
    ) -> OverlaySnapshot {
        let key = prompt.surface_key();
        let revision = self.overlay_revisions.current(key);
        let escape_dismissible = !matches!(prompt, SettingsOverlay::WebSearchConfigPopup(..))
            || !self.web_search_save_in_progress;
        let render_entity = entity.clone();
        let render_prompt = prompt.clone();
        let target_prompt = prompt.clone();
        let render_target = OverlayTarget::new(entity.clone(), move |this: &Self| {
            this.overlay_revisions.current(key) == revision && this.overlay_active(&target_prompt)
        });
        let presentation = SettingsOverlayPresentation::new(self);
        let dismiss_entity = entity.clone();
        let dismiss_prompt = prompt.clone();
        let mut snapshot = OverlaySnapshot::new(
            key,
            revision,
            move |exit_progress, viewport_height, cx| {
                render_entity.read(cx).render_owned_overlay(
                    render_target.clone(),
                    &presentation,
                    &render_prompt,
                    exit_progress,
                    viewport_height,
                    cx,
                )
            },
            move |window, cx| {
                dismiss_entity.update(cx, |this, cx| {
                    if this.overlay_revisions.current(key) != revision
                        || !this.overlay_active(&dismiss_prompt)
                    {
                        return;
                    }
                    this.dismiss_owned_overlay(&dismiss_prompt, window, cx);
                });
            },
        )
        .escape_dismissible(escape_dismissible);
        let focus = match &prompt {
            SettingsOverlay::SyncDirection(..) => None,
            SettingsOverlay::SyncPullConfirm(..) => None,
            SettingsOverlay::SyncUnsafeWriteConsent(..) => None,
            SettingsOverlay::LocalVaultDisableConfirm(..) => None,
            SettingsOverlay::LocalDataResetConfirm(..) => None,
            SettingsOverlay::SshBridgePolicyDowngrade(..) => None,
            SettingsOverlay::LocalDataResetConfirmationPopup(..) => Some(
                self.forms
                    .local_data_reset_confirmation_input
                    .focus_handle(cx),
            ),
            SettingsOverlay::SyncPassphraseClearConfirmPopup(..) => None,
            SettingsOverlay::SyncPassphrasePopup(..) => {
                Some(self.forms.sync_passphrase_input.focus_handle(cx))
            }
            SettingsOverlay::AiProviderPopup(..) => {
                Some(self.forms.ai_provider_name_input.focus_handle(cx))
            }
            SettingsOverlay::WebSearchConfigPopup(..) => {
                Some(self.forms.web_search_api_key_input.focus_handle(cx))
            }
            SettingsOverlay::SyncProviderConfigPopup(..) => None,
            SettingsOverlay::ProxyConfigPopup(..) => {
                Some(self.forms.proxy_name_input.focus_handle(cx))
            }
            SettingsOverlay::LocalVaultPassphrasePopup(..) => {
                Some(self.forms.local_vault_passphrase_input.focus_handle(cx))
            }
        };
        if let Some(focus) = focus {
            snapshot = snapshot.with_keyboard_focus(focus);
        }

        if matches!(
            prompt,
            SettingsOverlay::AiProviderPopup(..)
                | SettingsOverlay::WebSearchConfigPopup(..)
                | SettingsOverlay::SyncProviderConfigPopup(..)
                | SettingsOverlay::LocalVaultPassphrasePopup(..)
        ) {
            let submit_entity = entity.clone();
            let submit_prompt = prompt.clone();
            snapshot = snapshot.with_submit(move |window, cx| {
                submit_entity.update(cx, |this, cx| {
                    if this.overlay_revisions.current(key) != revision
                        || !this.overlay_active(&submit_prompt)
                    {
                        return;
                    }
                    match &submit_prompt {
                        SettingsOverlay::AiProviderPopup(..) => {
                            this.submit_ai_provider_save(window, cx);
                        }
                        SettingsOverlay::WebSearchConfigPopup(..) => {
                            this.submit_web_search_settings_save(window, cx);
                        }
                        SettingsOverlay::SyncProviderConfigPopup(..) => {
                            this.submit_sync_provider_config_popup_action(cx);
                        }
                        SettingsOverlay::LocalVaultPassphrasePopup(mode) => {
                            this.submit_local_vault_passphrase_popup_action(*mode, window, cx);
                        }
                        _ => {}
                    }
                });
            });
        }

        snapshot
    }

    fn render_owned_overlay(
        &self,
        entity: OverlayTarget<Self>,
        presentation: &SettingsOverlayPresentation,
        prompt: &SettingsOverlay,
        exit_progress: Option<f32>,
        viewport_height: f32,
        cx: &App,
    ) -> AnyElement {
        match prompt {
            SettingsOverlay::SyncDirection(prompt) => {
                self.render_sync_direction_prompt(entity, presentation, prompt, exit_progress)
            }
            SettingsOverlay::SyncPullConfirm(prompt) => {
                self.render_sync_pull_confirm_prompt(entity, presentation, prompt, exit_progress)
            }
            SettingsOverlay::SyncUnsafeWriteConsent(prompt) => self
                .render_sync_unsafe_write_consent_prompt(
                    entity,
                    presentation,
                    prompt,
                    exit_progress,
                ),
            SettingsOverlay::LocalVaultDisableConfirm(prompt) => self
                .render_local_vault_disable_confirm_prompt(
                    entity,
                    presentation,
                    prompt,
                    exit_progress,
                ),
            SettingsOverlay::LocalDataResetConfirm(prompt) => self
                .render_local_data_reset_confirm_prompt(
                    entity,
                    presentation,
                    prompt,
                    exit_progress,
                ),
            SettingsOverlay::SshBridgePolicyDowngrade(prompt) => self
                .render_ssh_bridge_policy_downgrade_prompt(
                    entity,
                    presentation,
                    prompt,
                    exit_progress,
                ),
            SettingsOverlay::LocalDataResetConfirmationPopup(prompt) => self
                .render_local_data_reset_confirmation_popup(
                    entity,
                    presentation,
                    *prompt,
                    exit_progress,
                    viewport_height,
                    cx,
                ),
            SettingsOverlay::SyncPassphraseClearConfirmPopup(prompt) => self
                .render_sync_passphrase_clear_confirm_popup(
                    entity,
                    presentation,
                    *prompt,
                    exit_progress,
                    viewport_height,
                    cx,
                ),
            SettingsOverlay::SyncPassphrasePopup(prompt) => self.render_sync_passphrase_popup(
                entity,
                presentation,
                *prompt,
                exit_progress,
                viewport_height,
                cx,
            ),
            SettingsOverlay::AiProviderPopup(prompt) => self.render_ai_provider_popup(
                entity,
                presentation,
                *prompt,
                exit_progress,
                viewport_height,
                cx,
            ),
            SettingsOverlay::WebSearchConfigPopup(prompt) => self.render_web_search_config_popup(
                entity,
                presentation,
                *prompt,
                exit_progress,
                viewport_height,
                cx,
            ),
            SettingsOverlay::SyncProviderConfigPopup(prompt) => self
                .render_sync_provider_config_popup(
                    entity,
                    presentation,
                    *prompt,
                    exit_progress,
                    viewport_height,
                    cx,
                ),
            SettingsOverlay::ProxyConfigPopup(prompt) => self.render_proxy_config_popup(
                entity,
                presentation,
                prompt.clone(),
                exit_progress,
                viewport_height,
                cx,
            ),
            SettingsOverlay::LocalVaultPassphrasePopup(prompt) => self
                .render_local_vault_passphrase_popup(
                    entity,
                    presentation,
                    *prompt,
                    exit_progress,
                    viewport_height,
                    cx,
                ),
        }
    }

    fn dismiss_owned_overlay(
        &mut self,
        prompt: &SettingsOverlay,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match prompt {
            SettingsOverlay::SyncDirection(..) => {
                self.cancel_sync_direction(cx);
            }
            SettingsOverlay::SyncPullConfirm(..) => {
                self.cancel_sync_pull_confirm(cx);
            }
            SettingsOverlay::SyncUnsafeWriteConsent(..) => {
                self.cancel_sync_unsafe_write_consent(cx);
            }
            SettingsOverlay::LocalVaultDisableConfirm(..) => {
                self.cancel_local_vault_disable_confirm(cx);
            }
            SettingsOverlay::LocalDataResetConfirm(..) => {
                self.cancel_local_data_reset_confirm(cx);
            }
            SettingsOverlay::SshBridgePolicyDowngrade(..) => {
                self.cancel_ssh_bridge_policy_downgrade(cx);
            }
            SettingsOverlay::LocalDataResetConfirmationPopup(..) => {
                self.close_local_data_reset_confirmation_popup(window, cx);
            }
            SettingsOverlay::SyncPassphraseClearConfirmPopup(..) => {
                self.close_sync_passphrase_clear_confirm_popup(cx);
            }
            SettingsOverlay::SyncPassphrasePopup(..) => {
                self.close_sync_passphrase_popup(window, cx);
            }
            SettingsOverlay::AiProviderPopup(..) => {
                self.close_ai_provider_popup(window, cx);
            }
            SettingsOverlay::WebSearchConfigPopup(..) => {
                self.close_web_search_config_popup(window, cx);
            }
            SettingsOverlay::SyncProviderConfigPopup(..) => {
                self.close_sync_provider_config_popup(window, cx);
            }
            SettingsOverlay::ProxyConfigPopup(..) => {
                self.close_proxy_config_popup(window, cx);
            }
            SettingsOverlay::LocalVaultPassphrasePopup(..) => {
                self.close_local_vault_passphrase_popup(window, cx);
            }
        }
    }
}

/// Retained render data has no route back to a replacement editor.
pub(super) struct SettingsOverlayPresentation {
    pub(super) forms: SettingsForms,
    settings: AppSettings,
    vault_status: LocalVaultStatus,
    vault_in_progress: bool,
    vault_primary_title: String,
    vault_change_title: String,
    sync_passphrase_label: String,
    sync_passphrase_operation: Option<SyncPassphraseOperation>,
    sync_provider_operation: Option<SyncProviderConfigSaveOperation>,
    ai_provider_id: Option<String>,
    ai_provider_save_in_progress: bool,
    ai_provider_key_load_target: Option<String>,
    web_search_save_in_progress: bool,
    secret_visibility: SecretVisibilityState,
    proxy_resolve_dns: bool,
    proxy_password_clear_requested: bool,
    proxies: Vec<ProxyProfile>,
}

impl SettingsOverlayPresentation {
    fn new(owner: &SettingsController) -> Self {
        Self {
            forms: owner.forms.clone(),
            settings: owner.settings_store.settings().clone(),
            vault_status: owner.local_vault_status,
            vault_in_progress: owner.local_vault_unlock_in_progress,
            vault_primary_title: owner.local_vault_primary_action_label(),
            vault_change_title: owner.local_vault_change_action_label(),
            sync_passphrase_label: owner.sync_passphrase_action_label(),
            sync_passphrase_operation: owner.sync.sync_passphrase_operation,
            sync_provider_operation: owner.sync.sync_provider_config_save_operation,
            ai_provider_id: owner.editing_ai_provider_id.clone(),
            ai_provider_save_in_progress: owner.ai_provider_save_in_progress,
            ai_provider_key_load_target: owner.ai_provider_api_key_load_in_progress.clone(),
            web_search_save_in_progress: owner.web_search_save_in_progress,
            secret_visibility: owner.secret_visibility.clone(),
            proxy_resolve_dns: owner.proxy_resolve_dns_through_proxy,
            proxy_password_clear_requested: owner.proxy_password_clear_requested,
            proxies: owner.proxies.clone(),
        }
    }
    pub(super) fn settings(&self) -> &AppSettings {
        &self.settings
    }
    pub(super) fn local_vault_status(&self) -> LocalVaultStatus {
        self.vault_status
    }
    pub(super) fn local_vault_unlock_in_progress(&self) -> bool {
        self.vault_in_progress
    }
    pub(super) fn local_vault_passphrase_popup_title(
        &self,
        mode: LocalVaultPassphrasePopupMode,
    ) -> String {
        match mode {
            LocalVaultPassphrasePopupMode::PrimaryAction => self.vault_primary_title.clone(),
            LocalVaultPassphrasePopupMode::ChangePassphrase => self.vault_change_title.clone(),
        }
    }
    pub(super) fn sync_passphrase_action_label(&self) -> String {
        self.sync_passphrase_label.clone()
    }
    pub(super) fn sync_passphrase_operation_in_progress(&self) -> bool {
        self.sync_passphrase_operation.is_some()
    }
    pub(super) fn sync_passphrase_save_in_progress(&self) -> bool {
        self.sync_passphrase_operation == Some(SyncPassphraseOperation::Save)
    }
    pub(super) fn sync_provider_config_save_in_progress_for(&self, provider: SyncProvider) -> bool {
        match provider {
            SyncProvider::GithubGist => {
                self.sync_provider_operation == Some(SyncProviderConfigSaveOperation::GithubGist)
            }
            SyncProvider::WebDav => {
                self.sync_provider_operation == Some(SyncProviderConfigSaveOperation::WebDav)
            }
            SyncProvider::None => false,
        }
    }
    pub(super) fn editing_ai_provider_id(&self) -> Option<&str> {
        self.ai_provider_id.as_deref()
    }
    pub(super) fn ai_provider_save_in_progress(&self) -> bool {
        self.ai_provider_save_in_progress
    }
    pub(super) fn ai_provider_api_key_load_in_progress_for(&self, provider: &str) -> bool {
        self.ai_provider_key_load_target.as_deref() == Some(provider)
    }
    pub(super) fn web_search_save_in_progress(&self) -> bool {
        self.web_search_save_in_progress
    }
    pub(super) fn secret_reveal_icon(&self, target: &SecretRevealTarget) -> AppIcon {
        if self.secret_visibility.is_visible(target) {
            AppIcon::EyeOff
        } else {
            AppIcon::Eye
        }
    }
    pub(super) fn selected_proxy_protocol(&self, cx: &App) -> ProxyProtocol {
        self.forms
            .proxy_protocol_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(ProxyProtocol::Socks5)
    }
    pub(super) fn selected_proxy_auth_mode(&self, cx: &App) -> ProxyAuthMode {
        self.forms
            .proxy_auth_mode_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(ProxyAuthMode::None)
    }
    pub(super) fn proxy_resolve_dns_through_proxy(&self) -> bool {
        self.proxy_resolve_dns
    }
    pub(super) fn proxy_password_clear_requested(&self) -> bool {
        self.proxy_password_clear_requested
    }
    pub(super) fn proxies(&self) -> &[ProxyProfile] {
        &self.proxies
    }
}
