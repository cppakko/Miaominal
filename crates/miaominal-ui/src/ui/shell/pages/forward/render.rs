use crate::ui::components::{
    SectionCard, SegmentedSwitch, editor_button, md3_select, md3_spinner, md3_switch,
};
use crate::ui::i18n;

use super::super::super::*;
use super::super::empty_state::shell_empty_page;
use super::{
    components::{forwarding_empty_state, forwarding_section},
    render_helpers::{
        ForwardRuleActions, build_forward_rule_context_menu, forward_rule_display_label,
        render_forward_endpoint_editor, route_direction_icon, truncate_with_ellipsis,
    },
};

const FORWARD_RULE_CARD_ACTION_WIDTH: f32 = 52.0;
const FORWARD_RULE_CARD_TITLE_MAX_CHARS: usize = 15;

#[derive(Clone, Copy)]
struct ForwardRuleConnectionUiState {
    session_active: bool,
    connected: bool,
    connecting: bool,
}

fn render_forward_rule_connection_control(
    actions: ForwardRuleActions,
    profile_id: String,
    rule_id: String,
    switch_id: String,
    state: ForwardRuleConnectionUiState,
) -> gpui::AnyElement {
    div()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(if state.connecting {
            div()
                .min_w(px(44.0))
                .min_h(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .child(md3_spinner(18.0))
                .into_any_element()
        } else {
            md3_switch(switch_id)
                .checked(state.connected)
                .tooltip(i18n::string("forwarding.tooltips.toggle_rule"))
                .on_click(move |enabled, _window, cx| {
                    let profile_id = profile_id.clone();
                    let rule_id = rule_id.clone();
                    actions.set_enabled(profile_id, rule_id, *enabled, cx);
                })
                .into_any_element()
        })
        .into_any_element()
}

fn render_forward_rule_card(
    controller: Entity<SessionController>,
    actions: ForwardRuleActions,
    profile: &SessionProfile,
    rule: &PortForwardRule,
    state: ForwardRuleConnectionUiState,
) -> impl IntoElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let extended = material.extended;
    let item_id = format!("forward-rule-card-{}-{}", profile.id, rule.id);
    let switch_id = format!("forward-rule-switch-{}-{}", profile.id, rule.id);
    let menu_profile_id = profile.id.clone();
    let menu_rule_id = rule.id.clone();
    let menu_listen_host = rule.listen_host.clone();
    let menu_listen_port = rule.listen_port;
    let menu_kind = rule.kind;
    let click_profile_id = profile.id.clone();
    let click_rule_id = rule.id.clone();
    let title = truncate_with_ellipsis(
        &forward_rule_display_label(rule),
        FORWARD_RULE_CARD_TITLE_MAX_CHARS,
    );
    let click_controller = controller.clone();
    let menu_controller = controller;
    let control = render_forward_rule_connection_control(
        actions.clone(),
        profile.id.clone(),
        rule.id.clone(),
        switch_id,
        state,
    );

    card_surface(roles.surface_container, 18.0)
        .id(item_id)
        .w(px(FORWARD_RULE_CARD_WIDTH))
        .min_h(px(FORWARD_RULE_CARD_HEIGHT))
        .border_1()
        .border_color(if state.connected {
            rgb(extended.info.color)
        } else {
            color_with_alpha(extended.info.color, 0x00)
        })
        .cursor_pointer()
        .p_4()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            let controller = click_controller.clone();
            let profile_id = click_profile_id.clone();
            let rule_id = click_rule_id.clone();
            controller.update(cx, |controller, cx| {
                controller.edit_port_forward_rule(profile_id.clone(), rule_id.clone(), window, cx);
            });
        })
        .context_menu(move |menu, _window, _cx| {
            build_forward_rule_context_menu(
                menu,
                menu_controller.clone(),
                actions.clone(),
                menu_profile_id.clone(),
                menu_rule_id.clone(),
                state.session_active,
                menu_listen_host.clone(),
                menu_listen_port,
                menu_kind,
            )
        })
        .child(
            h_flex()
                .size_full()
                .gap_3()
                .items_center()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .justify_center()
                        .gap_3()
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap_3()
                                .child(page_muted_icon_tile(AppIcon::Forward, 34.0, 10.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .text_size(miaominal_settings::FontSize::Heading.scaled())
                                        .text_color(rgb(roles.on_surface))
                                        .child(title),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(FORWARD_RULE_CARD_ACTION_WIDTH))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(control),
                ),
        )
}

fn render_forward_rule_list_row(
    controller: Entity<SessionController>,
    actions: ForwardRuleActions,
    profile: &SessionProfile,
    rule: &PortForwardRule,
    state: ForwardRuleConnectionUiState,
) -> impl IntoElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let extended = material.extended;
    let item_id = format!("forward-rule-row-{}-{}", profile.id, rule.id);
    let switch_id = format!("forward-rule-list-switch-{}-{}", profile.id, rule.id);
    let menu_profile_id = profile.id.clone();
    let menu_rule_id = rule.id.clone();
    let menu_listen_host = rule.listen_host.clone();
    let menu_listen_port = rule.listen_port;
    let menu_kind = rule.kind;
    let click_profile_id = profile.id.clone();
    let click_rule_id = rule.id.clone();
    let title = truncate_with_ellipsis(&forward_rule_display_label(rule), 42);
    let click_controller = controller.clone();
    let menu_controller = controller;
    let control = render_forward_rule_connection_control(
        actions.clone(),
        profile.id.clone(),
        rule.id.clone(),
        switch_id,
        state,
    );

    list_item_card(
        page_muted_icon_tile(AppIcon::Forward, 30.0, 10.0).into_any_element(),
        div()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .text_size(miaominal_settings::FontSize::Subheading.scaled())
            .text_color(rgb(roles.on_surface))
            .child(title)
            .into_any_element(),
        None,
        Some(
            h_flex()
                .items_center()
                .gap_2()
                .flex_shrink_0()
                .child(control)
                .into_any_element(),
        ),
        move |window, cx| {
            let controller = click_controller.clone();
            let profile_id = click_profile_id.clone();
            let rule_id = click_rule_id.clone();
            controller.update(cx, |controller, cx| {
                controller.edit_port_forward_rule(profile_id.clone(), rule_id.clone(), window, cx);
            });
        },
    )
    .id(item_id)
    .cursor_pointer()
    .context_menu(move |menu, _window, _cx| {
        build_forward_rule_context_menu(
            menu,
            menu_controller.clone(),
            actions.clone(),
            menu_profile_id.clone(),
            menu_rule_id.clone(),
            state.session_active,
            menu_listen_host.clone(),
            menu_listen_port,
            menu_kind,
        )
    })
    .border_1()
    .border_color(if state.connected {
        rgb(extended.info.color)
    } else {
        color_with_alpha(extended.info.color, 0x00)
    })
}

impl SessionController {
    fn render_port_forward_rule_composer(
        &self,
        controller: Entity<Self>,
        selected_profile: Option<&SessionProfile>,
    ) -> gpui::AnyElement {
        let forms = self.panel_forms().forwarding;
        let editor_state = self.editor_state();
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let profiles = self.profiles();
        let editing_rule = editor_state
            .port_forward_editor_profile_id
            .as_deref()
            .zip(editor_state.port_forward_editor_rule_id.as_deref())
            .and_then(|(profile_id, rule_id)| {
                profiles
                    .iter()
                    .find(|profile| profile.id == profile_id)
                    .and_then(|profile| {
                        profile
                            .port_forwarding_rules
                            .iter()
                            .find(|rule| rule.id == rule_id)
                    })
            });
        let is_editing_rule = editing_rule.is_some();
        let profile_badge = selected_profile
            .map(|profile| format!("{} rules", profile.port_forwarding_rules.len()));
        let online_badge = selected_profile.map(|profile| {
            format!(
                "{} online",
                self.online_session_count_for_profile(&profile.id)
            )
        });
        let forward_kind_selected_index = match editor_state.port_forward_kind {
            PortForwardKind::Local => 0,
            PortForwardKind::Remote => 1,
        };
        let (
            top_title,
            top_host_input,
            top_port_input,
            bottom_title,
            _bottom_copy,
            bottom_host_input,
            bottom_port_input,
        ) = match editor_state.port_forward_kind {
            PortForwardKind::Local => (
                i18n::string("forwarding.editor.listen_locally"),
                forms.listen_host_input.clone(),
                forms.listen_port_input.clone(),
                i18n::string("forwarding.editor.destination_behind_ssh_host"),
                i18n::string("forwarding.editor.forward_connections_copy"),
                forms.target_host_input.clone(),
                forms.target_port_input.clone(),
            ),
            PortForwardKind::Remote => (
                i18n::string("forwarding.editor.destination_on_this_machine"),
                forms.target_host_input.clone(),
                forms.target_port_input.clone(),
                i18n::string("forwarding.editor.expose_on_ssh_host"),
                i18n::string("forwarding.editor.ask_selected_host_copy"),
                forms.listen_host_input.clone(),
                forms.listen_port_input.clone(),
            ),
        };
        let profile_select = md3_select(&forms.profile_select)
            .large()
            .w_full()
            .rounded(px(14.0))
            .border_0()
            .bg(rgb(roles.surface_container_low))
            .icon(IconName::Search)
            .cleanable(!is_editing_rule)
            .search_placeholder(i18n::string("forwarding.editor.search_host_profiles"))
            .placeholder(if profiles.is_empty() {
                i18n::string("forwarding.editor.no_saved_host_profiles")
            } else {
                i18n::string("forwarding.editor.select_host_profile")
            })
            .disabled(profiles.is_empty() || is_editing_rule);
        let forward_kind_tabs = SegmentedSwitch::new("port-forward-editor-kind")
            .selected_index(forward_kind_selected_index)
            .width(260.0)
            .height(34.0)
            .padding(2.0)
            .item(i18n::string("forwarding.editor.local"))
            .item(i18n::string("forwarding.editor.remote"))
            .on_click({
                let controller = controller.clone();
                move |index, _, cx| {
                    let kind = match index {
                        0 => PortForwardKind::Local,
                        1 => PortForwardKind::Remote,
                        _ => return,
                    };
                    controller.update(cx, |controller, cx| {
                        controller.set_port_forward_kind(kind, cx);
                    });
                }
            });

        SectionCard::new(
            AppIcon::Forward,
            i18n::string("forwarding.editor.rule_configuration"),
            v_flex()
                .gap_4()
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .justify_between()
                        .gap_3()
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .when_some(profile_badge, |this, copy| {
                                    this.child(badge(
                                        copy,
                                        roles.surface_container_high,
                                        roles.on_surface_variant,
                                    ))
                                })
                                .when_some(online_badge, |this, copy| {
                                    this.child(badge(
                                        copy,
                                        roles.surface_container_low,
                                        roles.on_surface_variant,
                                    ))
                                }),
                        ),
                )
                .child(surface_text_input_stack(
                    i18n::string("forwarding.fields.label"),
                    forms.label_input.clone(),
                    TextInputSurface::Low,
                    false,
                ))
                .child(
                    v_flex()
                        .w_full()
                        .min_w(px(0.0))
                        .gap_4()
                        .child(
                            v_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(miaominal_settings::FontSize::Body.scaled())
                                        .text_color(rgb(roles.on_surface_variant))
                                        .child(i18n::string("forwarding.fields.mode")),
                                )
                                .child(forward_kind_tabs),
                        )
                        .child(render_forward_endpoint_editor(
                            top_title,
                            top_host_input,
                            top_port_input,
                        ))
                        .child(
                            v_flex()
                                .w_full()
                                .min_w(px(0.0))
                                .gap_2()
                                .child(
                                    h_flex()
                                        .w_full()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .size(px(28.0))
                                                .rounded(px(9.0))
                                                .bg(rgb(roles.surface_container_low))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .text_color(rgb(roles.on_surface_variant))
                                                .child(
                                                    Icon::new(route_direction_icon(
                                                        editor_state.port_forward_kind,
                                                    ))
                                                    .small(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(
                                                    miaominal_settings::FontSize::Body.scaled(),
                                                )
                                                .text_color(rgb(roles.on_surface_variant))
                                                .child(i18n::string(
                                                    "forwarding.fields.ssh_host_profile",
                                                )),
                                        ),
                                )
                                .child(profile_select)
                                .when(is_editing_rule, |this| {
                                    this.child(
                                        div()
                                            .text_size(miaominal_settings::FontSize::Body.scaled())
                                            .text_color(rgb(text_muted))
                                            .child(i18n::string(
                                                "forwarding.editor.profile_switching_disabled",
                                            )),
                                    )
                                }),
                        )
                        .child(render_forward_endpoint_editor(
                            bottom_title,
                            bottom_host_input,
                            bottom_port_input,
                        )),
                ),
        )
        .into_any_element()
    }

    pub(in crate::ui::shell) fn render_forward_fab(
        &self,
        controller: Entity<Self>,
    ) -> impl IntoElement {
        fab_button(move |window, cx| {
            controller.update(cx, |controller, cx| {
                controller.open_port_forward_panel(window, cx);
            });
        })
    }

    pub(in crate::ui::shell) fn render_port_forward_editor_sidebar(
        &self,
        controller: Entity<Self>,
        on_save: impl Fn(&mut Window, &mut App) + 'static,
        _cx: &App,
    ) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let editor_state = self.editor_state();
        let profiles = self.profiles();
        let selected_rule_label = editor_state
            .port_forward_editor_profile_id
            .as_deref()
            .zip(editor_state.port_forward_editor_rule_id.as_deref())
            .and_then(|(profile_id, rule_id)| {
                profiles
                    .iter()
                    .find(|profile| profile.id == profile_id)
                    .and_then(|profile| {
                        profile
                            .port_forwarding_rules
                            .iter()
                            .find(|rule| rule.id == rule_id)
                    })
            })
            .map(forward_rule_display_label);
        let is_editing_rule = selected_rule_label.is_some();
        let selected_rule_target = editor_state
            .port_forward_editor_profile_id
            .as_deref()
            .zip(editor_state.port_forward_editor_rule_id.as_deref())
            .map(|(profile_id, rule_id)| (profile_id.to_string(), rule_id.to_string()));
        let selected_profile = editor_state
            .port_forward_editor_profile_id
            .as_deref()
            .and_then(|profile_id| profiles.iter().find(|profile| profile.id == profile_id));

        let header = h_flex()
            .w_full()
            .items_start()
            .justify_between()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .gap_1()
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::PageTitle.scaled())
                            .text_color(rgb(roles.on_surface))
                            .child(if is_editing_rule {
                                i18n::string("forwarding.editor.edit_tunnel_rule")
                            } else {
                                i18n::string("forwarding.editor.add_tunnel_rule")
                            }),
                    )
                    .when_some(selected_rule_label, |this, label| {
                        this.child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(roles.on_surface))
                                .child(label),
                        )
                    }),
            );

        let mut footer_actions = Vec::new();
        if let Some((profile_id, rule_id)) = selected_rule_target {
            footer_actions.push(
                editor_button(
                    i18n::string("forwarding.actions.delete_rule"),
                    false,
                    true,
                    {
                        let controller = controller.clone();
                        move |_, cx| {
                            let profile_id = profile_id.clone();
                            let rule_id = rule_id.clone();
                            controller.update(cx, |controller, cx| {
                                controller.request_port_forward_rule_removal(
                                    &profile_id,
                                    &rule_id,
                                    cx,
                                );
                            })
                        }
                    },
                )
                .into_any_element(),
            );
        }
        footer_actions.push(
            editor_button(i18n::string("forwarding.actions.cancel"), false, true, {
                let controller = controller.clone();
                move |_, cx| {
                    controller.update(cx, |controller, cx| {
                        controller.close_port_forward_rule_editor(cx);
                    });
                }
            })
            .into_any_element(),
        );
        footer_actions.push(
            editor_button(
                if is_editing_rule {
                    i18n::string("forwarding.actions.save_rule")
                } else {
                    i18n::string("forwarding.actions.create_rule")
                },
                true,
                true,
                on_save,
            )
            .into_any_element(),
        );
        let footer = editor_footer_actions(footer_actions);

        div()
            .id("port-forward-editor-sidebar")
            .w(px(EDITOR_DRAWER_WIDTH))
            .min_w(px(360.0))
            .h_full()
            .bg(rgb(roles.surface_container))
            .child(
                v_flex()
                    .size_full()
                    .child(div().px_4().pt_4().pb_3().child(header))
                    .child(
                        div().flex_1().min_h_0().child(
                            div().size_full().overflow_y_scrollbar().child(
                                v_flex()
                                    .w_full()
                                    .px_4()
                                    .pb_4()
                                    .gap_3()
                                    .when(profiles.is_empty(), |this| {
                                        this.child(forwarding_empty_state(i18n::string(
                                            "forwarding.empty.no_hosts_available",
                                        )))
                                    })
                                    .when(!profiles.is_empty(), |this| {
                                        this.child(self.render_port_forward_rule_composer(
                                            controller.clone(),
                                            selected_profile,
                                        ))
                                    }),
                            ),
                        ),
                    )
                    .child(div().px_4().pt_3().pb_4().child(footer)),
            )
    }

    fn render_port_forward_saved_rules(
        &self,
        controller: Entity<Self>,
        actions: ForwardRuleActions,
        cx: &App,
    ) -> gpui::AnyElement {
        let filter_input = self.panel_forms().forwarding.filter_input;
        let filter_text = filter_input.read(cx).value().trim().to_ascii_lowercase();
        let profiles = self.profiles();
        let rules_with_profiles: Vec<_> = profiles
            .iter()
            .enumerate()
            .flat_map(|(profile_index, profile)| {
                profile
                    .port_forwarding_rules
                    .iter()
                    .filter(|rule| {
                        filter_text.is_empty()
                            || rule
                                .label
                                .trim()
                                .to_ascii_lowercase()
                                .contains(&filter_text)
                    })
                    .map(move |rule| (profile_index, profile, rule))
            })
            .collect();

        if rules_with_profiles.is_empty() {
            return forwarding_section(forwarding_empty_state(i18n::string(
                "forwarding.empty.no_filter_matches",
            )))
            .into_any_element();
        }

        let is_list = self.catalog_view().forward_view_mode == ProfileViewMode::List;
        let mut rules = if is_list {
            v_flex().w_full().gap_2()
        } else {
            div().flex().flex_wrap().gap_4()
        };

        for (_profile_index, profile, rule) in rules_with_profiles {
            let state = ForwardRuleConnectionUiState {
                session_active: self.has_port_forward_rule_session(&profile.id, &rule.id),
                connected: self.has_port_forward_rule_connection(&profile.id, &rule.id),
                connecting: self.is_port_forward_rule_connecting(&profile.id, &rule.id),
            };

            if is_list {
                rules = rules.child(render_forward_rule_list_row(
                    controller.clone(),
                    actions.clone(),
                    profile,
                    rule,
                    state,
                ));
            } else {
                rules = rules.child(render_forward_rule_card(
                    controller.clone(),
                    actions.clone(),
                    profile,
                    rule,
                    state,
                ));
            }
        }

        forwarding_section(
            v_flex()
                .gap_4()
                .child(page_section_title(i18n::string(
                    "forwarding.page.forwarding_rules",
                )))
                .child(rules),
        )
        .into_any_element()
    }

    pub(in crate::ui::shell) fn render_forward_page(
        &self,
        controller: Entity<Self>,
        cx: &App,
    ) -> gpui::AnyElement {
        let actions = ForwardRuleActions::new(controller.clone());
        let filter_input = self.panel_forms().forwarding.filter_input;
        let profiles = self.profiles();
        let total_rules = profiles
            .iter()
            .map(|profile| profile.port_forwarding_rules.len())
            .sum::<usize>();

        if profiles.is_empty() {
            return shell_empty_page(
                AppIcon::Forward,
                i18n::string("forwarding.empty.no_host_profiles"),
            )
            .into_any_element();
        }

        if total_rules == 0 {
            return shell_empty_page(AppIcon::Forward, i18n::string("forwarding.empty.no_rules"))
                .into_any_element();
        }

        let is_list = self.catalog_view().forward_view_mode == ProfileViewMode::List;

        let header =
            v_flex()
                .w_full()
                .gap_4()
                .px_5()
                .child(
                    h_flex().w_full().justify_center().child(
                        h_flex()
                            .w_full()
                            .max_w(px(576.0))
                            .child(search_filter_input(
                                &filter_input,
                                SearchInputStyle::Pill,
                                None,
                            )),
                    ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(page_view_mode_toolbar_item(AppIcon::Grid, !is_list, {
                            let controller = controller.clone();
                            move |_, cx| {
                                controller.update(cx, |controller, cx| {
                                    controller.set_forward_view_mode(ProfileViewMode::Grid, cx);
                                });
                            }
                        }))
                        .child(page_view_mode_toolbar_item(AppIcon::List, is_list, {
                            let controller = controller.clone();
                            move |_, cx| {
                                controller.update(cx, |controller, cx| {
                                    controller.set_forward_view_mode(ProfileViewMode::List, cx);
                                });
                            }
                        })),
                );

        div()
            .size_full()
            .overflow_hidden()
            .child(
                v_flex()
                    .size_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .gap_4()
                    .child(header)
                    .child(div().flex_1().min_w(px(0.0)).min_h(px(0.0)).child(
                        div().size_full().overflow_y_scrollbar().child(
                            v_flex().w_full().pb_8().child(
                                self.render_port_forward_saved_rules(controller, actions, cx),
                            ),
                        ),
                    )),
            )
            .into_any_element()
    }
}
