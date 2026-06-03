use super::super::super::*;
use super::super::empty_state::shell_empty_page;
use crate::ui::i18n;

const TRUSTED_CARD_ACTION_WIDTH: f32 = 30.0;

fn trusted_host_card(
    entity: Entity<AppView>,
    host: String,
    port: u16,
    algorithm: String,
) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let title = format!("{host}:{port}");
    let remove_host = host.clone();

    card_surface(roles.surface_container, 20.0)
        .w(px(TRUSTED_CARD_WIDTH))
        .min_h(px(TRUSTED_CARD_HEIGHT))
        .p_4()
        .child(
            h_flex()
                .size_full()
                .gap_1()
                .child(
                    div().flex_1().min_w(px(0.0)).h_full().child(
                        h_flex()
                            .w_full()
                            .h_full()
                            .items_center()
                            .gap_3()
                            .child(page_primary_icon_tile(AppIcon::FingerPrint, 44.0, 14.0))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .gap_4()
                                    .child(
                                        div()
                                            .min_w(px(0.0))
                                            .text_size(
                                                miaominal_settings::FontSize::Subtitle.scaled(),
                                            )
                                            .line_height(miaominal_settings::scaled_line_height(
                                                20.0,
                                            ))
                                            .text_color(rgb(roles.on_surface))
                                            .child(title),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .flex_wrap()
                                            .child(badge(
                                                algorithm,
                                                roles.surface_container_high,
                                                roles.on_surface_variant,
                                            ))
                                            .child(badge(
                                                i18n::string_args(
                                                    "trusted.page.port_badge",
                                                    &[("port", &port.to_string())],
                                                ),
                                                roles.surface_container_high,
                                                roles.on_surface_variant,
                                            )),
                                    ),
                            ),
                    ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(TRUSTED_CARD_ACTION_WIDTH))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon_button(
                            AppIcon::Close,
                            30.0,
                            10.0,
                            None,
                            None,
                            Some(roles.outline_variant),
                            move |_, cx| {
                                let host = remove_host.clone();
                                entity.update(cx, |this, cx| {
                                    this.request_trusted_known_host_removal(host, port, cx);
                                });
                            },
                        )),
                ),
        )
}

impl AppView {
    pub(in crate::ui::shell) fn render_trusted_known_host_delete_prompt(
        &self,
        entity: Entity<AppView>,
        prompt: &PendingKnownHostDeleteState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let subtitle = i18n::string_args(
            "dialogs.known_host_delete.message",
            &[
                ("host", prompt.host.as_str()),
                ("port", &prompt.port.to_string()),
            ],
        );

        let entity_cancel = entity.clone();
        let entity_confirm = entity.clone();

        let actions = h_flex()
            .gap_2()
            .justify_end()
            .child(
                basic_dialog_action_button(
                    "known-host-delete-cancel",
                    i18n::string("dialogs.known_host_delete.cancel"),
                    BasicDialogActionTone::Default,
                )
                .on_click(move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.cancel_trusted_known_host_removal(cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "known-host-delete-confirm",
                    i18n::string("dialogs.known_host_delete.confirm"),
                    BasicDialogActionTone::Destructive,
                )
                .on_click(move |_, _, cx| {
                    entity_confirm.update(cx, |this, cx| {
                        this.confirm_trusted_known_host_removal(cx);
                    });
                }),
            );

        render_basic_dialog(
            "known-host-delete",
            i18n::string("dialogs.known_host_delete.title"),
            Some(subtitle),
            None,
            actions.into_any_element(),
            exit_progress,
        )
    }

    pub(in crate::ui::shell) fn render_trusted_page(
        &self,
        entity: Entity<AppView>,
    ) -> gpui::AnyElement {
        let entries = self.data.known_hosts_entries.clone();
        let _path = self.services.known_hosts.path().display().to_string();

        if entries.is_empty() {
            return shell_empty_page(AppIcon::FingerPrint, i18n::string("trusted.page.empty"))
                .into_any_element();
        }

        div()
            .size_full()
            .child(
                div().size_full().overflow_y_scrollbar().child(
                    v_flex().w_full().gap_6().px_5().py_5().child(
                        v_flex()
                            .w_full()
                            .gap_4()
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(page_section_title(i18n::string(
                                        "trusted.page.saved_fingerprints",
                                    ))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_4()
                                    .children(entries.into_iter().map(|entry| {
                                        trusted_host_card(
                                            entity.clone(),
                                            entry.host,
                                            entry.port,
                                            entry.algorithm,
                                        )
                                        .into_any_element()
                                    }))
                                    .into_any_element(),
                            ),
                    ),
                ),
            )
            .into_any_element()
    }

    pub(in crate::ui::shell) fn render_trusted_host_key_prompt(
        &self,
        entity: Entity<AppView>,
        prompt: &HostKeyPrompt,
        exit_progress: Option<f32>,
        bottom_popup_viewport_height: f32,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let mismatch = prompt.previous_fingerprint.is_some();
        let title = if mismatch {
            i18n::string("session.status.host_key_mismatch")
        } else {
            i18n::string("session.status.verify_host_key")
        };
        let subtitle = if mismatch {
            let port = prompt.port.to_string();
            i18n::string_args(
                "trusted.prompt.mismatch_subtitle",
                &[("host", prompt.host.as_str()), ("port", &port)],
            )
        } else {
            let port = prompt.port.to_string();
            i18n::string_args(
                "trusted.prompt.verify_subtitle",
                &[("host", prompt.host.as_str()), ("port", &port)],
            )
        };

        let icon_tint = if mismatch {
            roles.error
        } else {
            material.extended.warning.color
        };

        let summary = h_flex()
            .w_full()
            .gap_4()
            .items_start()
            .child(
                div()
                    .size(px(52.0))
                    .rounded(px(16.0))
                    .bg(color_with_alpha(icon_tint, 0x28))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(icon_tint))
                    .child(Icon::new(AppIcon::FingerPrint).size(px(24.0))),
            )
            .child(
                v_flex().flex_1().min_w(px(0.0)).justify_center().child(
                    div()
                        .text_size(miaominal_settings::FontSize::Subheading.scaled())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(roles.on_surface))
                        .child(format!("{}:{}", prompt.host, prompt.port)),
                ),
            );

        let mut details = v_flex()
            .w_full()
            .gap_2()
            .child(detail_row(
                i18n::string("trusted.details.algorithm"),
                &prompt.algorithm,
            ))
            .child(detail_row(
                i18n::string("trusted.details.fingerprint_sha256"),
                &prompt.fingerprint,
            ));

        if let Some(previous) = prompt.previous_fingerprint.as_ref() {
            details = details.child(detail_row(
                i18n::string("trusted.details.previously_trusted"),
                previous,
            ));
        }

        let entity_once = entity.clone();
        let entity_save = entity.clone();
        let entity_reject = entity.clone();

        let actions = h_flex()
            .w_full()
            .justify_end()
            .gap_3()
            .child(
                basic_dialog_action_button(
                    "host-key-reject",
                    i18n::string("trusted.actions.reject"),
                    BasicDialogActionTone::Destructive,
                )
                .large()
                .on_click(move |_, _, cx| {
                    entity_reject.update(cx, |this, cx| {
                        this.handle_trusted_host_key_decision(HostKeyDecision::Reject, cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "host-key-once",
                    i18n::string("trusted.actions.accept_once"),
                    BasicDialogActionTone::Default,
                )
                .large()
                .on_click(move |_, _, cx| {
                    entity_once.update(cx, |this, cx| {
                        this.handle_trusted_host_key_decision(HostKeyDecision::AcceptOnce, cx);
                    });
                }),
            )
            .child(
                basic_dialog_action_button(
                    "host-key-save",
                    i18n::string("trusted.actions.trust_and_remember"),
                    BasicDialogActionTone::Default,
                )
                .large()
                .on_click(move |_, _, cx| {
                    entity_save.update(cx, |this, cx| {
                        this.handle_trusted_host_key_decision(HostKeyDecision::AcceptAndSave, cx);
                    });
                }),
            );

        let body = v_flex()
            .w_full()
            .gap_5()
            .child(
                div()
                    .w_full()
                    .rounded(px(18.0))
                    .bg(rgb(roles.surface_container_high))
                    .p_4()
                    .child(summary),
            )
            .child(
                div()
                    .w_full()
                    .rounded(px(18.0))
                    .bg(rgb(roles.surface_container_high))
                    .p_4()
                    .child(details),
            )
            .into_any_element();

        render_bottom_popup(
            bottom_popup_panel(
                title.to_string(),
                Some(subtitle),
                Some(body),
                actions.into_any_element(),
                bottom_popup_viewport_height,
            ),
            "trusted-host-key",
            exit_progress,
            |_window, _cx| {},
        )
    }
}

fn detail_row(label: String, value: &str) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );

    h_flex()
        .gap_3()
        .items_start()
        .child(
            div()
                .w(px(170.0))
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .text_color(rgb(text_muted))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .text_color(rgb(roles.on_surface))
                .child(value.to_string()),
        )
        .into_any_element()
}
