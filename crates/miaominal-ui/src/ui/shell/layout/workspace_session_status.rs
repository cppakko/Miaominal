use crate::ui::components::{fab_icon_button, md3_spinner};
use crate::ui::i18n;
use crate::ui::shell::state::SessionFailureStatus;
use gpui_component::Disableable;

use super::super::*;

struct SessionFailureView {
    title: String,
    summary: String,
    error: String,
    failure_status: Option<SessionFailureStatus>,
    profile_id: String,
    purpose: SessionPurpose,
    tab_id: usize,
}

pub(in crate::ui::shell::layout) fn session_summary(
    tab: &TabState,
    sessions: &[SessionProfile],
) -> String {
    let Some(session) = tab.as_session() else {
        return String::new();
    };

    if let Some(profile) = sessions
        .iter()
        .find(|profile| profile.id == session.profile_id)
    {
        return format!("{}@{}:{}", profile.username, profile.host, profile.port);
    }

    if let Some(profile) = session.pending_profile.as_ref() {
        return format!("{}@{}:{}", profile.username, profile.host, profile.port);
    }

    tab.title.clone()
}

impl AppView {
    fn hide_preserved_history_popup(&mut self, tab_id: usize, cx: &mut Context<Self>) {
        let Some(tab_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return;
        };
        let Some(session) = self.workspace_state.tabs[tab_index].as_session_mut() else {
            return;
        };

        session.hide_preserved_history_popup();
        cx.notify();
    }

    fn reconnect_session_tab(
        &mut self,
        tab_id: usize,
        profile_id: &str,
        write_marker: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return;
        };
        let profile = self
            .data
            .sessions
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned();

        if let Some(session) = self.workspace_state.tabs[tab_index].as_session_mut()
            && let Some(profile) = profile
        {
            session.commands = None;
            session.pending_profile = Some(profile);
            session.set_connection_state(SessionConnectionState::Connecting);
            session.reconnect_attempt = 0;
            if write_marker {
                session.terminal.push_text(&format!(
                    "{}\r\n",
                    i18n::string("session.terminal.reconnecting_marker")
                ));
            }
        }

        cx.notify();
    }

    pub(in crate::ui::shell::layout) fn render_session_placeholder(
        &self,
        tab: &TabState,
        rounded: bool,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let session = tab.as_session()?;
        if !session.uses_blocking_placeholder() {
            return None;
        }

        let summary = session_summary(tab, &self.data.sessions);

        match &session.connection_state {
            SessionConnectionState::Connecting => Some(self.render_session_connecting_surface(
                if session.purpose == SessionPurpose::PortForwarding {
                    i18n::string("session.workspace.connecting_forwarding_rule")
                } else {
                    i18n::string("session.workspace.connecting_to_host")
                },
                tab.status.clone(),
                summary,
                rounded,
            )),
            SessionConnectionState::Failed { error, status } => {
                Some(self.render_session_failure_surface(
                    SessionFailureView {
                        title: if session.purpose == SessionPurpose::PortForwarding {
                            i18n::string("session.workspace.forwarding_connection_failed")
                        } else {
                            i18n::string("session.workspace.connection_failed")
                        },
                        summary,
                        error: error.clone(),
                        failure_status: *status,
                        profile_id: session.profile_id.clone(),
                        purpose: session.purpose,
                        tab_id: tab.id,
                    },
                    rounded,
                    cx,
                ))
            }
            SessionConnectionState::Reconnecting { error, attempt } => {
                Some(self.render_session_reconnecting_surface(
                    summary,
                    error.clone(),
                    *attempt,
                    tab.id,
                    rounded,
                    cx,
                ))
            }
            SessionConnectionState::Ready => None,
            SessionConnectionState::Disconnected => Some(self.render_session_disconnected_surface(
                summary,
                session.profile_id.clone(),
                session.purpose,
                tab.id,
                rounded,
                cx,
            )),
        }
    }

    pub(in crate::ui::shell::layout) fn render_session_history_banner(
        &self,
        tab: &TabState,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let session = tab.as_session()?;
        if !session.preserves_terminal_history() {
            return None;
        }

        let summary = session_summary(tab, &self.data.sessions);
        let popup_hidden = session.preserved_history_popup_hidden();

        match &session.connection_state {
            SessionConnectionState::Failed { error, status } => Some(if popup_hidden {
                self.render_session_reconnect_fab(session.profile_id.clone(), true, tab.id, cx)
            } else {
                self.render_session_failure_banner(
                    SessionFailureView {
                        title: i18n::string("session.workspace.connection_failed"),
                        summary,
                        error: error.clone(),
                        failure_status: *status,
                        profile_id: session.profile_id.clone(),
                        purpose: session.purpose,
                        tab_id: tab.id,
                    },
                    cx,
                )
            }),
            SessionConnectionState::Disconnected => Some(if popup_hidden {
                self.render_session_reconnect_fab(session.profile_id.clone(), false, tab.id, cx)
            } else {
                self.render_session_disconnected_banner(
                    summary,
                    session.profile_id.clone(),
                    session.purpose,
                    tab.id,
                    cx,
                )
            }),
            SessionConnectionState::Connecting
            | SessionConnectionState::Ready
            | SessionConnectionState::Reconnecting { .. } => None,
        }
    }

    fn render_session_disconnected_surface(
        &self,
        summary: String,
        profile_id: String,
        purpose: SessionPurpose,
        tab_id: usize,
        rounded: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let weak = cx.entity().downgrade();
        let is_port_forward = purpose == SessionPurpose::PortForwarding;
        let profile_exists = self.data.sessions.iter().any(|p| p.id == profile_id);

        div()
            .size_full()
            .when(rounded, |this| this.rounded(px(16.0)))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(roles.background))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(560.0))
                    .items_center()
                    .gap_4()
                    .px_6()
                    .py_8()
                    .child(
                        div()
                            .size(px(56.0))
                            .rounded(px(18.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(color_with_alpha(text_muted, 0x18))
                            .child(
                                Icon::new(IconName::Minus)
                                    .large()
                                    .text_color(rgb(roles.on_surface_variant)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Display.scaled())
                            .text_color(rgb(roles.on_surface))
                            .child(i18n::string("session.workspace.session_closed")),
                    )
                    .when(!summary.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(summary),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_3()
                            .when(!is_port_forward && profile_exists, |this| {
                                this.child(icon_button(
                                    AppIcon::Rotate,
                                    36.0,
                                    12.0,
                                    Some(roles.primary),
                                    Some(roles.on_primary),
                                    None,
                                    {
                                        let weak = weak.clone();
                                        let profile_id = profile_id.clone();
                                        move |_window, cx| {
                                            weak.update(cx, |this, cx| {
                                                let Some(tab_index) = this
                                                    .workspace_state
                                                    .tabs
                                                    .iter()
                                                    .position(|t| t.id == tab_id)
                                                else {
                                                    return;
                                                };
                                                let profile = this
                                                    .data
                                                    .sessions
                                                    .iter()
                                                    .find(|p| p.id == profile_id)
                                                    .cloned();
                                                if let Some(session) = this.workspace_state.tabs
                                                    [tab_index]
                                                    .as_session_mut()
                                                    && let Some(profile) = profile
                                                {
                                                    session.commands = None;
                                                    session.pending_profile = Some(profile);
                                                    session.set_connection_state(
                                                        SessionConnectionState::Connecting,
                                                    );
                                                    session.reconnect_attempt = 0;
                                                }
                                                cx.notify();
                                            })
                                            .ok();
                                        }
                                    },
                                ))
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_session_connecting_surface(
        &self,
        title: String,
        status: String,
        summary: String,
        rounded: bool,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );

        div()
            .size_full()
            .when(rounded, |this| this.rounded(px(16.0)))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(roles.background))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(560.0))
                    .items_center()
                    .gap_4()
                    .px_6()
                    .py_8()
                    .child(md3_spinner(64.0))
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Display.scaled())
                            .text_color(rgb(roles.on_surface))
                            .child(title),
                    )
                    .when(!summary.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(summary),
                        )
                    })
                    .when(!status.is_empty(), |this| {
                        this.child(
                            div()
                                .w_full()
                                .px_4()
                                .py_3()
                                .rounded(px(16.0))
                                .bg(color_with_alpha(text_muted, 0x10))
                                .child(
                                    div()
                                        .text_size(miaominal_settings::FontSize::Input.scaled())
                                        .text_color(rgb(roles.on_surface_variant))
                                        .child(status),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_session_reconnecting_surface(
        &self,
        summary: String,
        error: String,
        attempt: u32,
        tab_id: usize,
        rounded: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        const MAX_RECONNECT_ATTEMPTS: u32 = 10;
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let weak = cx.entity().downgrade();
        let error_for_cancel = error.clone();
        div()
            .size_full()
            .when(rounded, |this| this.rounded(px(16.0)))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(roles.background))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(560.0))
                    .items_center()
                    .gap_4()
                    .px_6()
                    .py_8()
                    .child(md3_spinner(64.0))
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Display.scaled())
                            .text_color(rgb(roles.on_surface))
                            .child(i18n::string("session.workspace.reconnecting")),
                    )
                    .when(!summary.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(summary),
                        )
                    })
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Input.scaled())
                            .text_color(rgb(text_muted))
                            .child(i18n::string_args(
                                "session.workspace.reconnect_attempt",
                                &[
                                    ("attempt", &attempt.to_string()),
                                    ("max", &MAX_RECONNECT_ATTEMPTS.to_string()),
                                ],
                            )),
                    )
                    .child(
                        div().w_full().p_4().child(
                            div()
                                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                                .text_color(rgb(roles.on_surface))
                                .child(error.clone()),
                        ),
                    )
                    .child(icon_button(
                        AppIcon::Close,
                        36.0,
                        12.0,
                        None,
                        None,
                        None,
                        move |_window, cx| {
                            weak.update(cx, |this, cx| {
                                let Some(tab_index) = this
                                    .workspace_state
                                    .tabs
                                    .iter()
                                    .position(|t| t.id == tab_id)
                                else {
                                    return;
                                };
                                if let Some(session) =
                                    this.workspace_state.tabs[tab_index].as_session_mut()
                                {
                                    session.reconnect_task = None;
                                    session.reconnect_attempt = 0;
                                    session.set_connection_state(SessionConnectionState::Failed {
                                        error: error_for_cancel.clone(),
                                        status: None,
                                    });
                                }
                                cx.notify();
                            })
                            .ok();
                        },
                    )),
            )
            .into_any_element()
    }

    fn render_session_failure_surface(
        &self,
        failure: SessionFailureView,
        rounded: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let SessionFailureView {
            title,
            summary,
            error,
            failure_status,
            profile_id,
            purpose,
            tab_id,
        } = failure;

        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let weak = cx.entity().downgrade();
        let profile_id_retry = profile_id.clone();
        let profile_id_edit = profile_id.clone();
        let is_port_forward = purpose == SessionPurpose::PortForwarding;
        let profile_exists = self.data.sessions.iter().any(|p| p.id == profile_id);

        div()
            .size_full()
            .when(rounded, |this| this.rounded(px(16.0)))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(roles.background))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(620.0))
                    .items_center()
                    .gap_4()
                    .px_6()
                    .py_8()
                    .child(
                        div()
                            .size(px(56.0))
                            .rounded(px(18.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(color_with_alpha(roles.error, 0x18))
                            .child(
                                Icon::new(IconName::CircleX)
                                    .large()
                                    .text_color(rgb(roles.error)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Display.scaled())
                            .text_color(rgb(roles.on_surface))
                            .child(title),
                    )
                    .when(!summary.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(summary),
                        )
                    })
                    .when_some(failure_status, |this, failure_status| {
                        let status = match failure_status {
                            SessionFailureStatus::Closed => i18n::string("session.status.closed"),
                        };

                        this.child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(text_muted))
                                .child(status),
                        )
                    })
                    .child(
                        div().w_full().p_4().child(
                            div()
                                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                                .text_color(rgb(roles.on_surface))
                                .child(error),
                        ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .when(profile_exists, |this| {
                                this.child(icon_button(
                                    AppIcon::Rotate,
                                    36.0,
                                    12.0,
                                    Some(roles.primary),
                                    Some(roles.on_primary),
                                    None,
                                    {
                                        let weak = weak.clone();
                                        move |_window, cx| {
                                            weak.update(cx, |this, cx| {
                                                let Some(tab_index) = this
                                                    .workspace_state
                                                    .tabs
                                                    .iter()
                                                    .position(|t| t.id == tab_id)
                                                else {
                                                    return;
                                                };
                                                let profile = this
                                                    .data
                                                    .sessions
                                                    .iter()
                                                    .find(|p| p.id == profile_id_retry)
                                                    .cloned();
                                                if let Some(session) = this.workspace_state.tabs
                                                    [tab_index]
                                                    .as_session_mut()
                                                    && let Some(profile) = profile
                                                {
                                                    session.commands = None;
                                                    session.pending_profile = Some(profile);
                                                    session.set_connection_state(
                                                        SessionConnectionState::Connecting,
                                                    );
                                                    session.reconnect_attempt = 0;
                                                    session.terminal.push_text(&format!(
                                                        "{}\r\n",
                                                        i18n::string(
                                                            "session.terminal.reconnecting_marker"
                                                        )
                                                    ));
                                                }
                                                cx.notify();
                                            })
                                            .ok();
                                        }
                                    },
                                ))
                            })
                            .when(!is_port_forward && profile_exists, |this| {
                                this.child(icon_button(
                                    AppIcon::Edit,
                                    36.0,
                                    12.0,
                                    None,
                                    None,
                                    None,
                                    {
                                        let weak = weak.clone();
                                        move |window, cx| {
                                            weak.update(cx, |this, cx| {
                                                if let Some(index) = this
                                                    .data
                                                    .sessions
                                                    .iter()
                                                    .position(|p| p.id == profile_id_edit)
                                                {
                                                    this.open_hosts_tab(cx);
                                                    this.open_host_editor(index, window, cx);
                                                }
                                            })
                                            .ok();
                                        }
                                    },
                                ))
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_session_disconnected_banner(
        &self,
        summary: String,
        profile_id: String,
        purpose: SessionPurpose,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let weak = cx.entity().downgrade();
        let is_port_forward = purpose == SessionPurpose::PortForwarding;
        let profile_exists = self.data.sessions.iter().any(|p| p.id == profile_id);
        let supporting_text = (!summary.is_empty()).then_some(summary);

        let hide_action = {
            let weak = weak.clone();
            basic_dialog_action_button(
                SharedString::from(format!("session-hide-{tab_id}")),
                i18n::string("session.workspace.hide_action"),
                BasicDialogActionTone::Default,
            )
            .on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.hide_preserved_history_popup(tab_id, cx);
                })
                .ok();
            })
        };

        let reconnect_action = {
            let mut button = basic_dialog_action_button(
                SharedString::from(format!("session-reconnect-{tab_id}")),
                i18n::string("session.workspace.reconnect_action"),
                BasicDialogActionTone::Default,
            );
            button = button.disabled(is_port_forward || !profile_exists);
            if is_port_forward || !profile_exists {
                button = button.opacity(0.48);
            }

            let weak = weak.clone();
            button.on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.reconnect_session_tab(tab_id, &profile_id, false, cx);
                })
                .ok();
            })
        };

        let body = v_flex()
            .w_full()
            .min_w(px(0.0))
            .gap_3()
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .line_height(miaominal_settings::scaled_line_height(20.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string("session.terminal_messages.read_only_history")),
            )
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .gap_2()
            .justify_end()
            .child(hide_action)
            .child(reconnect_action)
            .into_any_element();

        render_basic_dialog_with_config(
            format!("session-disconnected-{tab_id}"),
            crate::ui::shell::support::BasicDialogConfig {
                title: i18n::string("session.workspace.session_closed"),
                supporting_text,
                body: Some(body),
                actions,
                icon: Some(BasicDialogIcon {
                    icon: AppIcon::Minimize,
                    tint: roles.on_surface_variant,
                }),
                header_alignment: BasicDialogHeaderAlignment::Center,
                exit_progress: None,
            },
        )
    }

    fn render_session_failure_banner(
        &self,
        failure: SessionFailureView,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let SessionFailureView {
            title,
            summary,
            error,
            failure_status,
            profile_id,
            purpose,
            tab_id,
        } = failure;

        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let weak = cx.entity().downgrade();
        let profile_id_retry = profile_id.clone();
        let is_port_forward = purpose == SessionPurpose::PortForwarding;
        let profile_exists = self.data.sessions.iter().any(|p| p.id == profile_id);
        let supporting_text = (!summary.is_empty()).then_some(summary);

        let hide_action = {
            let weak = weak.clone();
            basic_dialog_action_button(
                SharedString::from(format!("session-hide-{tab_id}")),
                i18n::string("session.workspace.hide_action"),
                BasicDialogActionTone::Default,
            )
            .on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.hide_preserved_history_popup(tab_id, cx);
                })
                .ok();
            })
        };

        let reconnect_action = {
            let mut button = basic_dialog_action_button(
                SharedString::from(format!("session-reconnect-{tab_id}")),
                i18n::string("session.workspace.reconnect_action"),
                BasicDialogActionTone::Default,
            );
            button = button.disabled(is_port_forward || !profile_exists);
            if is_port_forward || !profile_exists {
                button = button.opacity(0.48);
            }

            let weak = weak.clone();
            button.on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.reconnect_session_tab(tab_id, &profile_id_retry, true, cx);
                })
                .ok();
            })
        };

        let body = v_flex()
            .w_full()
            .min_w(px(0.0))
            .gap_3()
            .when_some(failure_status, |this, failure_status| {
                let status = match failure_status {
                    SessionFailureStatus::Closed => i18n::string("session.status.closed"),
                };

                this.child(
                    div()
                        .w_full()
                        .text_center()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .text_color(rgb(roles.on_surface_variant))
                        .child(status),
                )
            })
            .child(
                div()
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded(px(12.0))
                    .bg(color_with_alpha(roles.error, 0x10))
                    .child(
                        div()
                            .w_full()
                            .text_size(miaominal_settings::FontSize::Subheading.scaled())
                            .line_height(miaominal_settings::scaled_line_height(18.0))
                            .text_color(rgb(roles.on_surface))
                            .child(error),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .line_height(miaominal_settings::scaled_line_height(20.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string("session.terminal_messages.read_only_history")),
            )
            .into_any_element();

        let actions = h_flex()
            .w_full()
            .gap_2()
            .justify_end()
            .child(hide_action)
            .child(reconnect_action)
            .into_any_element();

        render_basic_dialog_with_config(
            format!("session-failure-{tab_id}"),
            crate::ui::shell::support::BasicDialogConfig {
                title,
                supporting_text,
                body: Some(body),
                actions,
                icon: Some(BasicDialogIcon {
                    icon: AppIcon::Close,
                    tint: roles.error,
                }),
                header_alignment: BasicDialogHeaderAlignment::Center,
                exit_progress: None,
            },
        )
    }

    fn render_session_reconnect_fab(
        &self,
        profile_id: String,
        write_marker: bool,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let weak = cx.entity().downgrade();

        div()
            .absolute()
            .right(px(20.0))
            .bottom(px(20.0))
            .child(fab_icon_button(AppIcon::Rotate, move |_window, cx| {
                weak.update(cx, |this, cx| {
                    this.reconnect_session_tab(tab_id, &profile_id, write_marker, cx);
                })
                .ok();
            }))
            .into_any_element()
    }
}
