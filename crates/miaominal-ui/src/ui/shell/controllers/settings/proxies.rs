use super::*;
use crate::ui::shell::{
    DeferredAppCommand, DialogOverlaySnapshot, SettingsDeferredCommand, ValidationFailure,
    error_notification, validation_notification,
};

#[derive(Debug)]
struct ProxySaveDraftInput {
    proxy_id: String,
    protocol: ProxyProtocol,
    auth_mode: ProxyAuthMode,
    name: String,
    host: String,
    port_text: String,
    username: String,
    password: String,
    resolve_dns_through_proxy: bool,
    password_clear_requested: bool,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct ProxySaveDraft {
    proxy: ProxyProfile,
    password_update: ProxyPasswordUpdate,
}

impl SettingsController {
    pub(in crate::ui::shell) fn proxy_management_picker_options(&self) -> Vec<(String, String)> {
        self.proxies
            .iter()
            .map(|proxy| (proxy.id.clone(), proxy_management_label(proxy)))
            .collect()
    }

    pub(in crate::ui::shell) fn selected_proxy_management_id(&self, cx: &App) -> Option<String> {
        self.forms
            .proxy_management_select
            .read(cx)
            .selected_value()
            .cloned()
    }

    pub(in crate::ui::shell) fn refresh_proxy_management_select(
        &mut self,
        preferred_proxy_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = proxy_management_select_options(&self.proxies);
        let selected_id = preferred_proxy_id
            .filter(|id| options.iter().any(|option| option.value().as_str() == *id))
            .map(ToOwned::to_owned)
            .or_else(|| options.first().map(|option| option.value().clone()));

        self.forms.proxy_management_select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            if let Some(selected_id) = selected_id.as_ref() {
                select.set_selected_value(selected_id, window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
    }

    pub(in crate::ui::shell) fn select_proxy_management(
        &mut self,
        proxy_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.proxies.iter().any(|proxy| proxy.id == proxy_id) {
            return;
        }
        self.forms.proxy_management_select.update(cx, |select, cx| {
            select.set_selected_value(&proxy_id.to_string(), window, cx);
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn selected_proxy_protocol(&self, cx: &App) -> ProxyProtocol {
        self.forms
            .proxy_protocol_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or_default()
    }

    pub(in crate::ui::shell) fn selected_proxy_auth_mode(&self, cx: &App) -> ProxyAuthMode {
        self.forms
            .proxy_auth_mode_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or_default()
    }

    pub(in crate::ui::shell) fn proxy_resolve_dns_through_proxy(&self) -> bool {
        self.proxy_resolve_dns_through_proxy
    }

    pub(in crate::ui::shell) fn proxy_password_clear_requested(&self) -> bool {
        self.proxy_password_clear_requested
    }

    pub(in crate::ui::shell) fn toggle_proxy_resolve_dns(&mut self, cx: &mut Context<Self>) {
        self.proxy_resolve_dns_through_proxy = !self.proxy_resolve_dns_through_proxy;
        cx.notify();
    }

    pub(in crate::ui::shell) fn begin_new_proxy(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PendingProxyConfigPopupState {
        let proxy = ProxyProfile::blank(ProxyService::next_proxy_id(), self.proxies.len() + 1);
        let popup = PendingProxyConfigPopupState {
            proxy_id: proxy.id.clone(),
            is_new: true,
        };
        self.load_proxy_form(proxy, window, cx);
        self.proxy_config_popup = Some(popup.clone());
        self.forms
            .proxy_name_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        popup
    }

    pub(in crate::ui::shell) fn begin_edit_proxy(
        &mut self,
        proxy_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingProxyConfigPopupState> {
        let Some(proxy) = self
            .proxies
            .iter()
            .find(|proxy| proxy.id == proxy_id)
            .cloned()
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "settings.proxies.messages.not_found",
            )));
            return None;
        };
        let popup = PendingProxyConfigPopupState {
            proxy_id: proxy.id.clone(),
            is_new: false,
        };
        self.load_proxy_form(proxy, window, cx);
        self.proxy_config_popup = Some(popup.clone());
        self.forms
            .proxy_name_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        Some(popup)
    }

    fn load_proxy_form(
        &mut self,
        proxy: ProxyProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_proxy_id = Some(proxy.id);
        self.proxy_resolve_dns_through_proxy = proxy.resolve_dns_through_proxy;
        self.proxy_password_clear_requested = false;
        set_input_value(&self.forms.proxy_name_input, proxy.name, window, cx);
        set_input_value(&self.forms.proxy_host_input, proxy.host, window, cx);
        set_input_value(
            &self.forms.proxy_port_input,
            proxy.port.to_string(),
            window,
            cx,
        );
        set_input_value(&self.forms.proxy_username_input, proxy.username, window, cx);
        set_input_value(&self.forms.proxy_password_input, String::new(), window, cx);
        self.forms.proxy_protocol_select.update(cx, |select, cx| {
            select.set_selected_value(&proxy.protocol, window, cx);
        });
        self.forms.proxy_auth_mode_select.update(cx, |select, cx| {
            select.set_selected_value(&proxy.auth_mode, window, cx);
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_proxy_config_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_proxy_id = None;
        self.proxy_password_clear_requested = false;
        set_input_value(&self.forms.proxy_password_input, String::new(), window, cx);
        if let Some(popup) = self.proxy_config_popup.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::ProxyConfigPopup(popup),
            ));
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn clear_proxy_password(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.proxy_password_clear_requested = true;
        set_input_value(&self.forms.proxy_password_input, String::new(), window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn save_proxy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(proxy_id) = self.editing_proxy_id.clone() else {
            return;
        };
        let draft = match self.proxy_save_draft(proxy_id, cx) {
            Ok(draft) => draft,
            Err(failure) => {
                self.notify_proxy_validation_failure(failure, window, cx);
                return;
            }
        };

        if self.local_vault_status == LocalVaultStatus::Locked {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::SaveProxy(draft),
            )));
            return;
        }

        self.continue_save_proxy_after_unlock(draft, window, cx);
    }

    pub(in crate::ui::shell) fn continue_save_proxy_after_unlock(
        &mut self,
        draft: ProxySaveDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_status == LocalVaultStatus::Locked {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::SaveProxy(draft),
            )));
            return;
        }

        let service = ProxyService::new(self.proxy_store.clone(), self.secrets.clone());
        let saved_proxy_id = draft.proxy.id.clone();
        match service.upsert(&mut self.proxies, draft.proxy, draft.password_update) {
            Ok(_) => {
                let proxies = self.proxies.clone();
                self.refresh_proxy_management_select(Some(&saved_proxy_id), window, cx);
                self.close_proxy_config_popup(window, cx);
                cx.emit(AppCommand::ProxiesChanged(proxies));
                cx.emit(AppCommand::Feedback(i18n::string(
                    "settings.proxies.messages.saved",
                )));
            }
            Err(error) => self.notify_proxy_save_failure(&error, window, cx),
        }
        cx.notify();
    }

    fn proxy_save_draft(
        &self,
        proxy_id: String,
        cx: &App,
    ) -> Result<ProxySaveDraft, ValidationFailure> {
        Self::validate_proxy_save_draft(
            &self.proxies,
            ProxySaveDraftInput {
                proxy_id,
                protocol: self.selected_proxy_protocol(cx),
                auth_mode: self.selected_proxy_auth_mode(cx),
                name: self.forms.proxy_name_input.read(cx).value().to_string(),
                host: self.forms.proxy_host_input.read(cx).value().to_string(),
                port_text: self.forms.proxy_port_input.read(cx).value().to_string(),
                username: self.forms.proxy_username_input.read(cx).value().to_string(),
                password: self.forms.proxy_password_input.read(cx).value().to_string(),
                resolve_dns_through_proxy: self.proxy_resolve_dns_through_proxy,
                password_clear_requested: self.proxy_password_clear_requested,
            },
        )
    }

    fn validate_proxy_save_draft(
        proxies: &[ProxyProfile],
        input: ProxySaveDraftInput,
    ) -> Result<ProxySaveDraft, ValidationFailure> {
        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "settings.proxies.validation.name_required",
            )));
        }
        if proxies.iter().any(|proxy| {
            proxy.id != input.proxy_id && proxy.name.trim().eq_ignore_ascii_case(&name)
        }) {
            return Err(ValidationFailure::invalid(i18n::string(
                "settings.proxies.validation.name_duplicate",
            )));
        }

        let host = input.host.trim().to_string();
        if host.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "settings.proxies.validation.host_required",
            )));
        }
        if host.chars().any(char::is_control) || host.chars().any(char::is_whitespace) {
            return Err(ValidationFailure::invalid(i18n::string(
                "settings.proxies.validation.host_invalid",
            )));
        }

        let port = input.port_text.trim().parse::<u16>().map_err(|_| {
            ValidationFailure::invalid(i18n::string("settings.proxies.messages.invalid_port"))
        })?;
        if port == 0 {
            return Err(ValidationFailure::invalid(i18n::string(
                "settings.proxies.messages.invalid_port",
            )));
        }

        let mut username = input.username.trim().to_string();
        let existing_has_password = proxies
            .iter()
            .find(|proxy| proxy.id == input.proxy_id)
            .is_some_and(|proxy| proxy.has_stored_password);
        if input.auth_mode == ProxyAuthMode::UsernamePassword {
            if username.is_empty() {
                return Err(ValidationFailure::required(i18n::string(
                    "settings.proxies.validation.username_required",
                )));
            }
            if input.protocol == ProxyProtocol::HttpConnect && username.contains(':') {
                return Err(ValidationFailure::invalid(i18n::string(
                    "settings.proxies.validation.http_username_colon",
                )));
            }
            if input.password.is_empty()
                && !existing_has_password
                && !input.password_clear_requested
            {
                return Err(ValidationFailure::required(i18n::string(
                    "settings.proxies.validation.password_required",
                )));
            }
        } else {
            username.clear();
        }

        let password_update = if input.auth_mode == ProxyAuthMode::None {
            ProxyPasswordUpdate::Clear
        } else if !input.password.is_empty() {
            ProxyPasswordUpdate::Set(input.password)
        } else if input.password_clear_requested {
            ProxyPasswordUpdate::Clear
        } else {
            ProxyPasswordUpdate::Keep
        };
        Ok(ProxySaveDraft {
            proxy: ProxyProfile {
                id: input.proxy_id,
                name,
                protocol: input.protocol,
                host,
                port,
                auth_mode: input.auth_mode,
                username,
                resolve_dns_through_proxy: input.resolve_dns_through_proxy,
                has_stored_password: existing_has_password,
            },
            password_update,
        })
    }

    fn notify_proxy_validation_failure(
        &mut self,
        failure: ValidationFailure,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = failure.message;
        crate::ui::shell::push_app_notification(
            window,
            validation_notification(failure.kind, message.clone()),
            cx,
        );
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn notify_proxy_save_failure(
        &mut self,
        error: &anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = i18n::string("settings.proxies.messages.save_failed");
        let detail = format!("{error:#}");
        crate::ui::shell::push_app_notification(
            window,
            error_notification(title.clone(), detail.clone()),
            cx,
        );
        cx.emit(AppCommand::Feedback(format!("{title}: {detail}")));
        cx.notify();
    }

    pub(in crate::ui::shell) fn delete_proxy(
        &mut self,
        proxy_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let service = ProxyService::new(self.proxy_store.clone(), self.secrets.clone());
        match service.delete(&mut self.proxies, proxy_id, &self.session_query.profiles()) {
            Ok(_) => {
                self.refresh_proxy_management_select(None, window, cx);
                cx.emit(AppCommand::ProxiesChanged(self.proxies.clone()));
                cx.emit(AppCommand::Feedback(i18n::string(
                    "settings.proxies.messages.deleted",
                )));
            }
            Err(error) => cx.emit(AppCommand::Feedback(format!(
                "{}: {error:#}",
                i18n::string("settings.proxies.messages.delete_failed")
            ))),
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::shell::ValidationNotificationKind;

    fn valid_input() -> ProxySaveDraftInput {
        ProxySaveDraftInput {
            proxy_id: "proxy-new".into(),
            protocol: ProxyProtocol::Socks5,
            auth_mode: ProxyAuthMode::None,
            name: "Local proxy".into(),
            host: "127.0.0.1".into(),
            port_text: "1080".into(),
            username: String::new(),
            password: String::new(),
            resolve_dns_through_proxy: true,
            password_clear_requested: false,
        }
    }

    #[test]
    fn proxy_save_validation_rejects_missing_and_invalid_fields() {
        let mut input = valid_input();
        input.name.clear();
        let error = SettingsController::validate_proxy_save_draft(&[], input)
            .expect_err("name should be required");
        assert_eq!(error.kind, ValidationNotificationKind::RequiredInputMissing);

        let mut input = valid_input();
        input.host = "proxy example".into();
        let error = SettingsController::validate_proxy_save_draft(&[], input)
            .expect_err("host whitespace should be invalid");
        assert_eq!(error.kind, ValidationNotificationKind::InvalidInput);

        let mut input = valid_input();
        input.port_text = "70000".into();
        let error = SettingsController::validate_proxy_save_draft(&[], input)
            .expect_err("out-of-range port should be invalid");
        assert_eq!(error.kind, ValidationNotificationKind::InvalidInput);
    }

    #[test]
    fn proxy_save_validation_rejects_duplicate_names_and_invalid_auth_fields() {
        let existing = ProxyProfile {
            id: "proxy-existing".into(),
            name: "Local Proxy".into(),
            protocol: ProxyProtocol::Socks5,
            host: "127.0.0.1".into(),
            port: 1080,
            auth_mode: ProxyAuthMode::None,
            username: String::new(),
            resolve_dns_through_proxy: true,
            has_stored_password: false,
        };
        let error = SettingsController::validate_proxy_save_draft(&[existing], valid_input())
            .expect_err("duplicate names should be rejected case-insensitively");
        assert_eq!(error.kind, ValidationNotificationKind::InvalidInput);

        let mut input = valid_input();
        input.protocol = ProxyProtocol::HttpConnect;
        input.auth_mode = ProxyAuthMode::UsernamePassword;
        input.username = "user:name".into();
        input.password = "secret".into();
        let error = SettingsController::validate_proxy_save_draft(&[], input)
            .expect_err("HTTP proxy usernames cannot contain colons");
        assert_eq!(error.kind, ValidationNotificationKind::InvalidInput);

        let mut input = valid_input();
        input.auth_mode = ProxyAuthMode::UsernamePassword;
        input.username = "alice".into();
        let error = SettingsController::validate_proxy_save_draft(&[], input)
            .expect_err("new authenticated proxies should require a password");
        assert_eq!(error.kind, ValidationNotificationKind::RequiredInputMissing);
    }
}
