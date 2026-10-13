use super::*;

impl SettingsController {
    /// A reopened editor owns new input entities. Exit snapshots retain the old
    /// handles and cannot display or edit a later submission.
    pub(super) fn renew_popup_forms(
        &mut self,
        key: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fields: Vec<(&mut Entity<InputState>, bool)> = match key {
            "ai-provider" => vec![
                (&mut self.forms.ai_provider_name_input, false),
                (&mut self.forms.ai_provider_model_input, false),
                (&mut self.forms.ai_provider_base_url_input, false),
                (&mut self.forms.ai_provider_api_key_input, true),
                (&mut self.forms.ai_provider_temperature_input, false),
                (&mut self.forms.ai_provider_max_tokens_input, false),
                (&mut self.forms.ai_provider_context_window_input, false),
            ],
            "web-search-config" => vec![
                (&mut self.forms.web_search_api_key_input, true),
                (&mut self.forms.web_search_endpoint_input, false),
                (&mut self.forms.web_search_max_results_input, false),
            ],
            "proxy-config" => vec![
                (&mut self.forms.proxy_name_input, false),
                (&mut self.forms.proxy_host_input, false),
                (&mut self.forms.proxy_port_input, false),
                (&mut self.forms.proxy_username_input, false),
                (&mut self.forms.proxy_password_input, true),
            ],
            "sync-provider-config" => vec![
                (&mut self.forms.sync_github_token_input, true),
                (&mut self.forms.sync_github_gist_id_input, false),
                (&mut self.forms.sync_webdav_url_input, false),
                (&mut self.forms.sync_webdav_username_input, false),
                (&mut self.forms.sync_webdav_password_input, true),
            ],
            "sync-passphrase" => vec![
                (&mut self.forms.sync_passphrase_input, true),
                (&mut self.forms.sync_passphrase_confirmation_input, true),
            ],
            "local-vault-passphrase" => vec![
                (&mut self.forms.local_vault_current_passphrase_input, true),
                (&mut self.forms.local_vault_passphrase_input, true),
                (
                    &mut self.forms.local_vault_passphrase_confirmation_input,
                    true,
                ),
            ],
            "local-data-reset-confirmation" => {
                vec![(&mut self.forms.local_data_reset_confirmation_input, false)]
            }
            _ => Vec::new(),
        };
        for (field, masked) in fields {
            *field = new_input_state("", "", masked, window, cx);
        }
        let mut subscriptions = Vec::new();
        match key {
            "ai-provider" => {
                let options = AiProviderKind::all()
                    .iter()
                    .copied()
                    .filter(|kind| ai_provider_kind_chat_supported(*kind))
                    .map(|kind| {
                        SelectOption::new(kind, i18n::string(ai_provider_kind_label_key(kind)))
                    })
                    .collect::<Vec<_>>();
                self.forms.ai_provider_kind_select =
                    cx.new(|cx| SelectState::new(options, None, window, cx));
                subscriptions.push(cx.subscribe(
                    &self.forms.ai_provider_kind_select,
                    |this: &mut Self, source, event, cx| {
                        if source.entity_id() != this.forms.ai_provider_kind_select.entity_id() {
                            return;
                        }
                        let SelectEvent::Confirm(selected) = event;
                        if let Some(kind) = selected.as_ref().copied()
                            && this.editing_ai_provider_id.is_none()
                        {
                            cx.emit(AppCommand::Feedback(i18n::string_args(
                                "settings.ai_providers.status.kind_selected",
                                &[("kind", &i18n::string(ai_provider_kind_label_key(kind)))],
                            )));
                            cx.notify();
                        }
                    },
                ));
            }
            "web-search-config" => {
                let options = WebSearchProviderKind::all()
                    .iter()
                    .copied()
                    .map(|kind| {
                        SelectOption::new(
                            kind,
                            i18n::string(web_search_provider_kind_label_key(kind)),
                        )
                    })
                    .collect::<Vec<_>>();
                self.forms.web_search_kind_select =
                    cx.new(|cx| SelectState::new(options, None, window, cx));
                subscriptions.push(cx.subscribe(
                    &self.forms.web_search_kind_select,
                    |this: &mut Self, source, event, cx| {
                        if source.entity_id() != this.forms.web_search_kind_select.entity_id() {
                            return;
                        }
                        let SelectEvent::Confirm(selected) = event;
                        if let Some(kind) = selected.as_ref().copied() {
                            this.on_web_search_kind_changed(kind, cx);
                        }
                    },
                ));
            }
            "proxy-config" => {
                let protocols = vec![
                    SelectOption::new(ProxyProtocol::Socks5, "SOCKS5"),
                    SelectOption::new(ProxyProtocol::HttpConnect, "HTTP CONNECT"),
                ];
                let auth_modes = vec![
                    SelectOption::new(
                        ProxyAuthMode::None,
                        i18n::string("settings.proxies.auth.none"),
                    ),
                    SelectOption::new(
                        ProxyAuthMode::UsernamePassword,
                        i18n::string("settings.proxies.auth.username_password"),
                    ),
                ];
                self.forms.proxy_protocol_select =
                    cx.new(|cx| SelectState::new(protocols, None, window, cx));
                self.forms.proxy_auth_mode_select =
                    cx.new(|cx| SelectState::new(auth_modes, None, window, cx));
                subscriptions.push(cx.subscribe(
                    &self.forms.proxy_protocol_select,
                    |this: &mut Self, source, event, cx| {
                        if source.entity_id() != this.forms.proxy_protocol_select.entity_id() {
                            return;
                        }
                        let SelectEvent::Confirm(selected) = event;
                        if this.editing_proxy_id.is_some()
                            && let Some(protocol) = selected.as_ref().copied()
                        {
                            let port_input = this.forms.proxy_port_input.clone();
                            let current_port = port_input.read(cx).value().trim().to_string();
                            if matches!(current_port.as_str(), "1080" | "8080")
                                && let Some(window_handle) = cx.active_window()
                            {
                                let port = protocol.default_port().to_string();
                                let _ = window_handle.update(cx, move |_, window, cx| {
                                    set_input_value(&port_input, port, window, cx);
                                });
                            }
                            cx.notify();
                        }
                    },
                ));
                subscriptions.push(cx.subscribe(
                    &self.forms.proxy_auth_mode_select,
                    |this: &mut Self, source, event, cx| {
                        if source.entity_id() != this.forms.proxy_auth_mode_select.entity_id() {
                            return;
                        }
                        let SelectEvent::Confirm(selected) = event;
                        if this.editing_proxy_id.is_some() && selected.is_some() {
                            cx.notify();
                        }
                    },
                ));
            }
            _ => {}
        }
        self.popup_form_subscriptions.insert(key, subscriptions);
        self.refresh_localized_placeholders(window, cx);
    }
}

pub(super) fn new_popup_input(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("").masked(true));
    crate::ui::components::register_input_hint(&input, i18n::string(placeholder), cx);
    input
}
