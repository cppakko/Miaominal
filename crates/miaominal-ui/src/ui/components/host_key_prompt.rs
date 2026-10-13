//! Shared host-key decision surface for SSH sessions and SFTP connections.

use crate::ui::assets::AppIcon;
use crate::ui::components::animation::render_bottom_popup;
use crate::ui::components::tokens::color_with_alpha;
use crate::ui::components::{
    BasicDialogActionTone, basic_dialog_action_button, bottom_popup_panel,
};
use crate::ui::i18n;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{App, FontWeight, div, prelude::*, px, rgb};
use miaominal_ssh::{HostKeyDecision, HostKeyPrompt};

pub(in crate::ui) fn render_host_key_prompt(
    on_decision: impl Fn(HostKeyDecision, &mut App) + Clone + 'static,
    prompt: &HostKeyPrompt,
    exit_progress: Option<f32>,
    bottom_popup_viewport_height: f32,
    stable_key: impl AsRef<str>,
) -> gpui_kit::AnyElement {
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

    let on_reject = on_decision.clone();
    let on_once = on_decision.clone();
    let on_save = on_decision;

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
                on_reject(HostKeyDecision::Reject, cx);
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
                on_once(HostKeyDecision::AcceptOnce, cx);
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
                on_save(HostKeyDecision::AcceptAndSave, cx);
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
        stable_key,
        exit_progress,
        |_window, _cx| {},
    )
}

fn detail_row(label: String, value: &str) -> gpui_kit::AnyElement {
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
                .w(px(132.0))
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .text_color(rgb(text_muted))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .line_height(miaominal_settings::scaled_line_height(18.0))
                .text_color(rgb(roles.on_surface))
                .child(value.to_string()),
        )
        .into_any_element()
}
