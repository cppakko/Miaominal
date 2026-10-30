use crate::ui::assets::AppIcon;
use crate::ui::components::animation::{
    BasicDialogConfig, render_basic_dialog, render_basic_dialog_with_config,
};
use crate::ui::components::overlay_target::OverlayTarget;
use crate::ui::components::tokens::color_with_alpha;
use crate::ui::components::{
    BasicDialogActionTone, BasicDialogHeaderAlignment, BasicDialogIcon, TextInputSurface,
    basic_dialog_action_button, field_label, surface_text_input,
};
use crate::ui::contracts::{overlay::OverlaySnapshot, workspace::TabId};
use crate::ui::features::session::SessionController;
use crate::ui::i18n;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{
    Disableable,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    radio::{Radio, RadioGroup},
    scroll::ScrollableElement,
    v_flex,
};
use gpui_kit::{AnyElement, Entity, FontWeight, SharedString, div, prelude::*, px, rgb};
use miaominal_services::{RdpConnectionMode, RdpRuntimeState, RdpSessionSnapshot};

pub(in crate::ui::features::session) fn launch_overlay(
    entity: Entity<SessionController>,
    revision: u64,
) -> OverlaySnapshot {
    let target = OverlayTarget::new(entity.clone(), move |owner| {
        owner.is_rdp_launch_current(revision)
    });
    let dismiss = target.clone();
    let submit = target.clone();
    OverlaySnapshot::new(
        "rdp-launch",
        revision,
        move |exit, _, cx| {
            let Some(draft) = entity.read(cx).rdp_launch_draft() else {
                return div().into_any_element();
            };
            let mode_target = target.clone();
            let reuse_target = target.clone();
            let launch_target = target.clone();
            let cancel_target = target.clone();
            let roles = miaominal_settings::current_theme().material.roles;
            let selected = usize::from(draft.mode == RdpConnectionMode::Direct);
            let (port_input, port) = {
                let owner = entity.read(cx);
                (owner.rdp_port_input(), owner.rdp_launch_port(cx))
            };
            let shown_port = port
                .unwrap_or(miaominal_services::RdpLaunchOptions::DEFAULT_PORT)
                .to_string();
            let options = [
                (
                    "rdp-mode-tunnel",
                    i18n::string("rdp.tunnel"),
                    i18n::string_args(
                        "rdp.tunnel_supporting",
                        &[("target", &draft.ssh_target), ("port", &shown_port)],
                    ),
                ),
                (
                    "rdp-mode-direct",
                    i18n::string("rdp.direct"),
                    i18n::string_args(
                        "rdp.direct_supporting",
                        &[("host", &draft.host), ("port", &shown_port)],
                    ),
                ),
            ];
            // Connection mode: one exclusive choice whose consequences need a sentence each,
            // so a radio list with supporting text rather than a segmented button.
            let mode = RadioGroup::vertical("rdp-mode")
                .selected_index(Some(selected))
                .children(options.into_iter().enumerate().map(
                    |(index, (id, headline, supporting))| {
                        let checked = index == selected;
                        Radio::new(id)
                            .label(headline)
                            .w_full()
                            .px_4()
                            .py_3()
                            .rounded(px(OPTION_RADIUS))
                            .border_1()
                            .text_size(miaominal_settings::FontSize::Title.scaled())
                            .text_color(rgb(if checked {
                                roles.on_secondary_container
                            } else {
                                roles.on_surface
                            }))
                            .map(|row| {
                                if checked {
                                    row.bg(rgb(roles.secondary_container))
                                        .border_color(rgb(roles.secondary_container))
                                } else {
                                    row.border_color(rgb(roles.outline_variant)).hover(|row| {
                                        row.bg(color_with_alpha(roles.on_surface, 0x14))
                                    })
                                }
                            })
                            .child(supporting_text(
                                supporting,
                                if checked {
                                    roles.on_secondary_container
                                } else {
                                    roles.on_surface_variant
                                },
                            ))
                    },
                ))
                .on_change(move |index, _, cx| {
                    mode_target.update(cx, |owner, cx| {
                        owner.set_rdp_launch(
                            Some(if *index == 0 {
                                RdpConnectionMode::SshTunnel
                            } else {
                                RdpConnectionMode::Direct
                            }),
                            None,
                            cx,
                        )
                    });
                });
            // Reusing the password is an independent choice that applies on Connect, so a
            // checkbox (a switch would imply an immediate effect). When it cannot apply it is
            // disabled and the reason replaces its supporting text.
            let reuse_available = draft.reuse_unavailable.is_none();
            let reuse_supporting = match draft.reuse_unavailable {
                Some(reason) => i18n::string(reason),
                None => i18n::string_args("rdp.reuse_supporting", &[("user", &draft.username)]),
            };
            let reuse = Checkbox::new("rdp-reuse-password")
                .accessibility_label(i18n::string("rdp.reuse_password"))
                .checked(draft.reuse && reuse_available)
                .disabled(!reuse_available)
                .text_size(miaominal_settings::FontSize::Title.scaled())
                // Rendered as a child so its color follows MD3 rather than the component's
                // muted tone: disabled content is on-surface at 38% opacity. The reason below
                // stays fully readable because it explains how to make the option available.
                .child(
                    div()
                        .text_color(if reuse_available {
                            rgb(roles.on_surface)
                        } else {
                            color_with_alpha(roles.on_surface, 0x61)
                        })
                        .child(i18n::string("rdp.reuse_password")),
                )
                .when(reuse_available, |checkbox| {
                    checkbox.tooltip(i18n::string("rdp.password_hint"))
                })
                .child(supporting_text(reuse_supporting, roles.on_surface_variant))
                .on_click(move |reuse, _, cx| {
                    reuse_target
                        .update(cx, |owner, cx| owner.set_rdp_launch(None, Some(*reuse), cx));
                });
            // MD3 text field: label, field, then supporting text that turns into the error
            // message (with an error outline) when the value is not a port.
            let port_field = port_input.map(|input| {
                let invalid = port.is_none();
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(field_label(i18n::string("rdp.port_label"), false))
                    .child(
                        div().id("rdp-port-input").test_support().w_40().child(
                            surface_text_input(&input, TextInputSurface::Highest)
                                .when(invalid, |field| {
                                    field.border_1().border_color(rgb(roles.error))
                                }),
                        ),
                    )
                    .child(supporting_text(
                        i18n::string(if invalid {
                            "rdp.port_invalid"
                        } else {
                            "rdp.port_supporting"
                        }),
                        if invalid {
                            roles.error
                        } else {
                            roles.on_surface_variant
                        },
                    ))
            });
            let body = v_flex()
                .w_full()
                .min_w(px(0.0))
                .gap_6()
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(section_label(i18n::string("rdp.mode_label"), roles))
                        .child(mode)
                        .when_some(port_field, |section, field| section.child(field.mt_2())),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_3()
                        .child(section_label(i18n::string("rdp.credentials_label"), roles))
                        .child(reuse),
                )
                .into_any_element();
            let actions = h_flex()
                .w_full()
                .gap_2()
                .justify_end()
                .child(
                    basic_dialog_action_button(
                        "rdp-launch-cancel",
                        i18n::string("dialogs.common.cancel"),
                        BasicDialogActionTone::Default,
                    )
                    .on_click(move |_, _, cx| {
                        cancel_target.update(cx, |owner, cx| owner.dismiss_rdp_launch(cx));
                    }),
                )
                .child(
                    basic_dialog_action_button(
                        "rdp-launch-submit",
                        i18n::string("rdp.connect"),
                        BasicDialogActionTone::Default,
                    )
                    .on_click(move |_, _, cx| {
                        launch_target.update(cx, |owner, cx| owner.submit_rdp_launch(cx));
                    }),
                )
                .into_any_element();
            render_basic_dialog_with_config(
                "rdp-launch-dialog",
                BasicDialogConfig {
                    title: i18n::string("rdp.launch_title"),
                    supporting_text: Some(draft.name),
                    body: Some(body),
                    actions,
                    icon: Some(BasicDialogIcon {
                        icon: AppIcon::Computer,
                        tint: roles.primary,
                    }),
                    header_alignment: BasicDialogHeaderAlignment::Center,
                    exit_progress: exit,
                },
            )
        },
        move |_, cx| {
            dismiss.update(cx, |owner, cx| owner.dismiss_rdp_launch(cx));
        },
    )
    .with_submit(move |_, cx| {
        submit.update(cx, |owner, cx| owner.submit_rdp_launch(cx));
    })
}

/// MD3 medium shape for the selectable connection options.
const OPTION_RADIUS: f32 = 12.0;

/// MD3 title-small section label inside the dialog body.
fn section_label(label: String, roles: miaominal_settings::theme::Md3Roles) -> AnyElement {
    div()
        .w_full()
        .text_size(miaominal_settings::FontSize::Heading.scaled())
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(roles.on_surface_variant))
        .child(label)
        .into_any_element()
}

/// MD3 body-medium supporting line beneath an option's headline.
fn supporting_text(text: String, color: u32) -> AnyElement {
    div()
        .w_full()
        .min_w(px(0.0))
        .text_size(miaominal_settings::FontSize::Heading.scaled())
        .line_height(miaominal_settings::scaled_line_height(20.0))
        .text_color(rgb(color))
        .child(text)
        .into_any_element()
}

pub(in crate::ui::features::session) fn render_status(
    entity: Entity<SessionController>,
    tab_id: TabId,
    runtime: &RdpSessionSnapshot,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let running = runtime.state().is_active();
    let ready =
        runtime.local_port().is_some() && !matches!(runtime.state(), RdpRuntimeState::Reconnecting);
    let mut actions = h_flex().flex_wrap().gap_2();
    for (id, key, enabled) in [
        ("reopen", "rdp.reopen", ready),
        (
            "hold",
            if runtime.keep_tunnel() {
                "rdp.release_hold"
            } else {
                "rdp.hold"
            },
            running,
        ),
        ("stop", "rdp.disconnect", running),
        ("restart", "rdp.reconnect", !running),
    ] {
        let entity = entity.clone();
        actions = actions.child(
            Button::new(SharedString::from(format!("rdp-{id}-{tab_id}")))
                .label(i18n::string(key))
                .ghost()
                .disabled(!enabled)
                .on_click(move |_, _, cx| {
                    entity.update(cx, |owner, cx| {
                        owner.rdp_action(tab_id, id, cx);
                    })
                }),
        );
    }
    v_flex()
        .size_full()
        .min_h_0()
        .p_6()
        .gap_4()
        .bg(rgb(roles.background))
        .text_color(rgb(roles.on_surface))
        .child(
            div()
                .text_lg()
                .child(format!("RDP · {}", runtime.profile_name())),
        )
        .child(div().child(super::super::controller::rdp::status(runtime)))
        .when_some(runtime.local_port(), |view, port| {
            let host = if runtime.local_host().is_empty() {
                "127.0.0.1"
            } else {
                runtime.local_host()
            };
            view.child(
                div()
                    .text_sm()
                    .text_color(rgb(roles.on_surface_variant))
                    .child(format!(
                        "{host}:{port} → 127.0.0.1:{}",
                        runtime.remote_port()
                    )),
            )
        })
        .when_some(runtime.release_in(), |view, seconds| {
            view.child(i18n::string_args(
                "rdp.release_in",
                &[("seconds", &seconds.to_string())],
            ))
        })
        .when(runtime.keep_tunnel(), |view| {
            view.child(i18n::string("rdp.held"))
        })
        .when(runtime.reopen_required(), |view| {
            view.child(
                div()
                    .text_color(rgb(roles.error))
                    .child(i18n::string("rdp.port_changed")),
            )
        })
        .child(actions)
        .child(
            div()
                .text_sm()
                .text_color(rgb(roles.on_surface_variant))
                .child(i18n::string("rdp.logs")),
        )
        .child(
            div()
                .id(SharedString::from(format!("rdp-logs-{tab_id}")))
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(
                    v_flex().gap_1().children(
                        runtime
                            .log()
                            .iter()
                            .map(|line| div().text_sm().child(line.clone())),
                    ),
                ),
        )
        .into_any_element()
}

pub(in crate::ui::features::session) fn confirmation_overlay(
    entity: Entity<SessionController>,
    revision: u64,
) -> OverlaySnapshot {
    let target = OverlayTarget::new(entity, move |owner| {
        owner.is_rdp_confirmation_current(revision)
    });
    let dismiss = target.clone();
    let submit = target.clone();
    OverlaySnapshot::new(
        "rdp-disconnect",
        revision,
        move |exit, _, _| {
            let confirm = target.clone();
            let cancel = target.clone();
            render_basic_dialog(
                "rdp-confirm-dialog",
                i18n::string("rdp.confirm_title"),
                Some(i18n::string("rdp.confirm_message")),
                None,
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        basic_dialog_action_button(
                            "rdp-confirm-cancel",
                            i18n::string("dialogs.common.cancel"),
                            BasicDialogActionTone::Default,
                        )
                        .on_click(move |_, _, cx| {
                            cancel.update(cx, |owner, cx| owner.finish_rdp_confirmation(false, cx));
                        }),
                    )
                    .child(
                        basic_dialog_action_button(
                            "rdp-confirm-disconnect",
                            i18n::string("rdp.disconnect"),
                            BasicDialogActionTone::Destructive,
                        )
                        .on_click(move |_, _, cx| {
                            confirm.update(cx, |owner, cx| owner.finish_rdp_confirmation(true, cx));
                        }),
                    )
                    .into_any_element(),
                exit,
            )
        },
        move |_, cx| {
            dismiss.update(cx, |owner, cx| owner.finish_rdp_confirmation(false, cx));
        },
    )
    .with_submit(move |_, cx| {
        submit.update(cx, |owner, cx| owner.finish_rdp_confirmation(true, cx));
    })
}
