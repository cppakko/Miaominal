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
use crate::ui::contracts::overlay::OverlaySnapshot;
use crate::ui::features::session::{RdpAction, SessionController};
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
            // MD3 text field: label and field; an error message (with an error outline)
            // appears only when the value is not a port.
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
                    .when(invalid, |field| {
                        field.child(supporting_text(
                            i18n::string("rdp.port_invalid"),
                            roles.error,
                        ))
                    })
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

impl SessionController {
    /// Footer status-center section: the remote desktops this window launched, with their
    /// commands. `None` when there are none, so the center shows notifications only.
    pub(in crate::ui) fn render_rdp_status_section(
        &self,
        entity: Entity<Self>,
    ) -> Option<AnyElement> {
        let sessions = self.rdp_sessions();
        if sessions.is_empty() {
            return None;
        }
        let roles = miaominal_settings::current_theme().material.roles;
        let has_ended = sessions.iter().any(|runtime| !runtime.state().is_active());
        let clear = entity.clone();
        Some(
            v_flex()
                .id("rdp-panel")
                .test_support()
                .w_full()
                .gap_2()
                .text_color(rgb(roles.on_surface))
                .child(
                    h_flex()
                        .w_full()
                        .min_h(px(32.0))
                        .px_2()
                        .justify_between()
                        .child(crate::ui::components::notifications::section_label(
                            i18n::string("rdp.title"),
                            roles,
                        ))
                        .when(has_ended, |row| {
                            row.child(
                                Button::new("rdp-clear-ended")
                                    .label(i18n::string("rdp.clear_ended"))
                                    .ghost()
                                    .compact()
                                    .rounded(px(20.0))
                                    .on_click(move |_, _, cx| {
                                        clear.update(cx, |owner, cx| owner.clear_ended_rdp(cx));
                                    }),
                            )
                        }),
                )
                .child(
                    // The popover has no definite height, so a flex-filled list would
                    // collapse to nothing; the list sizes to its rows and scrolls past a limit.
                    div()
                        .id("rdp-panel-list")
                        .w_full()
                        .max_h(px(LIST_MAX_HEIGHT))
                        .overflow_y_scrollbar()
                        .child(
                            v_flex().gap_1().children(
                                sessions
                                    .iter()
                                    .map(|runtime| self.render_rdp_row(&entity, runtime, roles)),
                            ),
                        ),
                )
                .into_any_element(),
        )
    }

    /// One remote desktop: a status dot, its name, a one-line summary and the main actions.
    /// Technical detail (port mapping, keep-tunnel, explanations, log) sits behind Details.
    fn render_rdp_row(
        &self,
        entity: &Entity<Self>,
        runtime: &RdpSessionSnapshot,
        roles: miaominal_settings::theme::Md3Roles,
    ) -> AnyElement {
        let id = runtime.id().to_string();
        let tunnel = runtime.mode() == RdpConnectionMode::SshTunnel;
        let running = runtime.state().is_active();
        let failed = matches!(runtime.state(), RdpRuntimeState::Failed(_));
        let ready = runtime.local_port().is_some()
            && !matches!(runtime.state(), RdpRuntimeState::Reconnecting);
        let details_open = self.is_rdp_log_open(&id);

        // Summary: state, then the one fact that changes what happens next.
        let mut summary = super::super::controller::rdp::status(runtime);
        if failed {
            // The full error is in the log under Details.
            summary = i18n::string("rdp.failed_short");
        } else if running && tunnel {
            if runtime.keep_tunnel() {
                summary = format!("{summary} · {}", i18n::string("rdp.held"));
            } else if let Some(seconds) = runtime.release_in() {
                let release =
                    i18n::string_args("rdp.release_in", &[("seconds", &seconds.to_string())]);
                summary = format!("{summary} · {release}");
            }
        }
        let dot = if failed {
            roles.error
        } else if running {
            roles.primary
        } else {
            roles.outline
        };

        let action = |key: &'static str, label: String, enabled: bool, action: RdpAction| {
            let entity = entity.clone();
            let id = id.clone();
            Button::new(SharedString::from(format!("rdp-{key}-{id}")))
                .label(label)
                .ghost()
                .compact()
                .disabled(!enabled)
                .on_click(move |_, _, cx| {
                    entity.update(cx, |owner, cx| owner.rdp_action(&id, action, cx));
                })
        };
        let mut buttons = h_flex().flex_wrap().gap_1();
        if running && tunnel {
            buttons = buttons
                .child(action(
                    "reopen",
                    i18n::string("rdp.reopen"),
                    ready,
                    RdpAction::Reopen,
                ))
                .child(action(
                    "stop",
                    i18n::string("rdp.disconnect"),
                    true,
                    RdpAction::Disconnect,
                ));
        } else if !running {
            buttons = buttons
                .child(action(
                    "restart",
                    i18n::string("rdp.reconnect"),
                    true,
                    RdpAction::Reconnect,
                ))
                .child(action(
                    "remove",
                    i18n::string("rdp.remove"),
                    true,
                    RdpAction::Remove,
                ));
        }
        let toggle_entity = entity.clone();
        let toggle_id = id.clone();
        buttons = buttons.child(
            Button::new(SharedString::from(format!("rdp-log-{id}")))
                .label(i18n::string(if details_open {
                    "rdp.hide_log"
                } else {
                    "rdp.show_log"
                }))
                .ghost()
                .compact()
                .on_click(move |_, _, cx| {
                    toggle_entity.update(cx, |owner, cx| owner.toggle_rdp_log(&toggle_id, cx));
                }),
        );

        let details = details_open.then(|| {
            let mut lines = Vec::new();
            if let Some(port) = runtime.local_port() {
                let host = if runtime.local_host().is_empty() {
                    "127.0.0.1"
                } else {
                    runtime.local_host()
                };
                lines.push(format!(
                    "{host}:{port} → 127.0.0.1:{}",
                    runtime.remote_port()
                ));
            }
            if runtime.client_handed_off() {
                lines.push(i18n::string("rdp.handed_off"));
            }
            v_flex()
                .w_full()
                .gap_1()
                .pt_1()
                .text_xs()
                .text_color(rgb(roles.on_surface_variant))
                .children(lines.into_iter().map(|line| div().child(line)))
                .when(running && tunnel, |details| {
                    details.child(h_flex().child(action(
                        "hold",
                        i18n::string(if runtime.keep_tunnel() {
                            "rdp.release_hold"
                        } else {
                            "rdp.hold"
                        }),
                        true,
                        RdpAction::ToggleHold,
                    )))
                })
                .when(!runtime.log().is_empty(), |details| {
                    details.child(
                        div()
                            .id(SharedString::from(format!("rdp-log-lines-{id}")))
                            .max_h(px(LOG_MAX_HEIGHT))
                            .overflow_y_scrollbar()
                            .child(v_flex().gap_1().children(
                                runtime.log().iter().map(|line| div().child(line.clone())),
                            )),
                    )
                })
        });

        v_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_1()
            .rounded(px(ROW_RADIUS))
            .bg(rgb(roles.surface_container_high))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(div().flex_none().size(px(8.0)).rounded_full().bg(rgb(dot)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(runtime.profile_name().to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(rgb(roles.on_surface_variant))
                            .child(i18n::string(if tunnel {
                                "rdp.tunnel"
                            } else {
                                "rdp.direct"
                            })),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(rgb(if failed {
                        roles.error
                    } else {
                        roles.on_surface_variant
                    }))
                    .child(summary),
            )
            .when(runtime.reopen_required(), |row| {
                row.child(
                    div()
                        .text_sm()
                        .text_color(rgb(roles.error))
                        .child(i18n::string("rdp.port_changed")),
                )
            })
            .child(buttons)
            .children(details)
            .into_any_element()
    }
}

/// Matches the notification history entries below the section.
const ROW_RADIUS: f32 = 18.0;
/// Session list height before it scrolls, leaving room for notifications below.
const LIST_MAX_HEIGHT: f32 = 240.0;
const LOG_MAX_HEIGHT: f32 = 160.0;

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
