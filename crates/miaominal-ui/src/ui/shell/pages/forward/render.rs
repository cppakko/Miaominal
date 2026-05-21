use crate::ui::components::{SectionCard, editor_button, md3_spinner, md3_switch};
use crate::ui::i18n;
use gpui_component::{
    Size,
    tab::{Tab, TabBar},
};

use super::super::super::*;
use super::super::empty_state::shell_empty_page;
use super::{
    components::{forwarding_empty_state, forwarding_section},
    render_helpers::{
        build_forward_rule_context_menu, forward_rule_display_label,
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
    entity: Entity<AppView>,
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
            let switch_entity = entity;
            md3_switch(switch_id)
                .checked(state.connected)
                .tooltip(i18n::string("forwarding.tooltips.toggle_rule"))
                .on_click(move |enabled, _window, cx| {
                    let entity = switch_entity.clone();
                    let profile_id = profile_id.clone();
                    let rule_id = rule_id.clone();
                    entity.update(cx, |this, cx| {
                        this.set_port_forward_rule_enabled(&profile_id, &rule_id, *enabled, cx);
                    });
                })
                .into_any_element()
        })
        .into_any_element()
}

fn render_forward_rule_card(
    entity: Entity<AppView>,
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
    let click_profile_id = profile.id.clone();
    let click_rule_id = rule.id.clone();
    let title = truncate_with_ellipsis(
        &forward_rule_display_label(rule),
        FORWARD_RULE_CARD_TITLE_MAX_CHARS,
    );
    let click_entity = entity.clone();
    let menu_entity = entity.clone();
    let control = render_forward_rule_connection_control(
        entity,
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
            let entity = click_entity.clone();
            let profile_id = click_profile_id.clone();
            let rule_id = click_rule_id.clone();
            entity.update(cx, |this, cx| {
                this.edit_port_forward_rule(profile_id.clone(), rule_id.clone(), window, cx);
            });
        })
        .context_menu(move |menu, _window, _cx| {
            build_forward_rule_context_menu(
                menu,
                menu_entity.clone(),
                menu_profile_id.clone(),
                menu_rule_id.clone(),
                state.session_active,
            )
        })
        .child(
            h_flex()
                .size_full()
                .gap_3()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .justify_between()
                        .gap_3()
                        .child(
                            h_flex()
                                .w_full()
                                .items_start()
                                .gap_3()
                                .child(page_muted_icon_tile(AppIcon::Forward, 34.0, 10.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .text_size(miaominal_settings::scaled_font_size(14.0))
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
    entity: Entity<AppView>,
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
    let click_profile_id = profile.id.clone();
    let click_rule_id = rule.id.clone();
    let title = truncate_with_ellipsis(&forward_rule_display_label(rule), 42);
    let click_entity = entity.clone();
    let menu_entity = entity.clone();
    let control = render_forward_rule_connection_control(
        entity,
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
            .text_size(miaominal_settings::scaled_font_size(13.0))
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
            let entity = click_entity.clone();
            let profile_id = click_profile_id.clone();
            let rule_id = click_rule_id.clone();
            entity.update(cx, |this, cx| {
                this.edit_port_forward_rule(profile_id.clone(), rule_id.clone(), window, cx);
            });
        },
    )
    .id(item_id)
    .cursor_pointer()
    .context_menu(move |menu, _window, _cx| {
        build_forward_rule_context_menu(
            menu,
            menu_entity.clone(),
            menu_profile_id.clone(),
            menu_rule_id.clone(),
            state.session_active,
        )
    })
    .border_1()
    .border_color(if state.connected {
        rgb(extended.info.color)
    } else {
        color_with_alpha(extended.info.color, 0x00)
    })
}

impl AppView {
    fn render_port_forward_rule_composer(
        &self,
        entity: Entity<Self>,
        selected_profile: Option<&SessionProfile>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let editing_rule = self
            .editors
            .port_forward_editor_profile_id
            .as_deref()
            .zip(self.editors.port_forward_editor_rule_id.as_deref())
            .and_then(|(profile_id, rule_id)| {
                self.data
                    .sessions
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
        let forward_kind_selected_index = match self.editors.port_forward_kind {
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
        ) = match self.editors.port_forward_kind {
            PortForwardKind::Local => (
                i18n::string("forwarding.editor.listen_locally"),
                self.panel_forms.forwarding.listen_host_input.clone(),
                self.panel_forms.forwarding.listen_port_input.clone(),
                i18n::string("forwarding.editor.destination_behind_ssh_host"),
                i18n::string("forwarding.editor.forward_connections_copy"),
                self.panel_forms.forwarding.target_host_input.clone(),
                self.panel_forms.forwarding.target_port_input.clone(),
            ),
            PortForwardKind::Remote => (
                i18n::string("forwarding.editor.destination_on_this_machine"),
                self.panel_forms.forwarding.target_host_input.clone(),
                self.panel_forms.forwarding.target_port_input.clone(),
                i18n::string("forwarding.editor.expose_on_ssh_host"),
                i18n::string("forwarding.editor.ask_selected_host_copy"),
                self.panel_forms.forwarding.listen_host_input.clone(),
                self.panel_forms.forwarding.listen_port_input.clone(),
            ),
        };
        let profile_select = Select::new(&self.panel_forms.forwarding.profile_select)
            .large()
            .w_full()
            .rounded(px(14.0))
            .border_0()
            .bg(rgb(roles.surface_container_low))
            .icon(IconName::Search)
            .cleanable(!is_editing_rule)
            .search_placeholder(i18n::string("forwarding.editor.search_host_profiles"))
            .placeholder(if self.data.sessions.is_empty() {
                i18n::string("forwarding.editor.no_saved_host_profiles")
            } else {
                i18n::string("forwarding.editor.select_host_profile")
            })
            .disabled(self.data.sessions.is_empty() || is_editing_rule);
        let forward_kind_tabs = TabBar::new("port-forward-editor-kind")
            .w_full()
            .pill()
            .with_size(Size::Small)
            .selected_index(forward_kind_selected_index)
            .on_click({
                let entity = entity.clone();
                move |index, _, cx| {
                    let kind = match *index {
                        0 => PortForwardKind::Local,
                        1 => PortForwardKind::Remote,
                        _ => return,
                    };
                    entity.update(cx, |this, cx| {
                        this.set_port_forward_kind(kind, cx);
                    });
                }
            })
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("forwarding.editor.local")),
            )
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("forwarding.editor.remote")),
            );

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
                    self.panel_forms.forwarding.label_input.clone(),
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
                                        .text_size(miaominal_settings::scaled_font_size(11.0))
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
                                                        self.editors.port_forward_kind,
                                                    ))
                                                    .small(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(miaominal_settings::scaled_font_size(
                                                    11.0,
                                                ))
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
                                            .text_size(miaominal_settings::scaled_font_size(11.0))
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
        entity: Entity<Self>,
    ) -> impl IntoElement {
        fab_button(move |window, cx| {
            entity.update(cx, |this, cx| this.open_port_forward_panel(window, cx));
        })
    }

    pub(in crate::ui::shell) fn render_port_forward_editor_sidebar(
        &self,
        entity: Entity<Self>,
        _cx: &App,
    ) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let selected_rule_label = self
            .editors
            .port_forward_editor_profile_id
            .as_deref()
            .zip(self.editors.port_forward_editor_rule_id.as_deref())
            .and_then(|(profile_id, rule_id)| {
                self.data
                    .sessions
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
        let selected_rule_target = self
            .editors
            .port_forward_editor_profile_id
            .as_deref()
            .zip(self.editors.port_forward_editor_rule_id.as_deref())
            .map(|(profile_id, rule_id)| (profile_id.to_string(), rule_id.to_string()));
        let selected_profile = self
            .editors
            .port_forward_editor_profile_id
            .as_deref()
            .and_then(|profile_id| {
                self.data
                    .sessions
                    .iter()
                    .find(|profile| profile.id == profile_id)
            });

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
                            .text_size(miaominal_settings::scaled_font_size(20.0))
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
                                .text_size(miaominal_settings::scaled_font_size(12.0))
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
                        let entity = entity.clone();
                        move |_, cx| {
                            let profile_id = profile_id.clone();
                            let rule_id = rule_id.clone();
                            entity.update(cx, |this, cx| {
                                this.request_port_forward_rule_removal(&profile_id, &rule_id, cx);
                            })
                        }
                    },
                )
                .into_any_element(),
            );
        }
        footer_actions.push(
            editor_button(i18n::string("forwarding.actions.cancel"), false, true, {
                let entity = entity.clone();
                move |_, cx| {
                    entity.update(cx, |this, cx| {
                        this.close_port_forward_rule_editor(cx);
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
                {
                    let entity = entity.clone();
                    move |window, cx| {
                        entity.update(cx, |this, cx| {
                            this.create_port_forward_rule(window, cx);
                        });
                    }
                },
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
                                    .when(self.data.sessions.is_empty(), |this| {
                                        this.child(forwarding_empty_state(i18n::string(
                                            "forwarding.empty.no_hosts_available",
                                        )))
                                    })
                                    .when(!self.data.sessions.is_empty(), |this| {
                                        this.child(self.render_port_forward_rule_composer(
                                            entity.clone(),
                                            selected_profile,
                                        ))
                                    }),
                            ),
                        ),
                    )
                    .child(div().px_4().pt_3().pb_4().child(footer)),
            )
    }

    fn render_port_forward_saved_rules(&self, entity: Entity<Self>, cx: &App) -> gpui::AnyElement {
        let filter_text = self
            .panel_forms
            .forwarding
            .filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let rules_with_profiles: Vec<_> = self
            .data
            .sessions
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

        let is_list = self.panel_view.forward_view_mode == ProfileViewMode::List;
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
                    entity.clone(),
                    profile,
                    rule,
                    state,
                ));
            } else {
                rules = rules.child(render_forward_rule_card(
                    entity.clone(),
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
        entity: Entity<Self>,
        cx: &App,
    ) -> gpui::AnyElement {
        let total_rules = self
            .data
            .sessions
            .iter()
            .map(|profile| profile.port_forwarding_rules.len())
            .sum::<usize>();

        if self.data.sessions.is_empty() {
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

        let is_list = self.panel_view.forward_view_mode == ProfileViewMode::List;

        let header =
            v_flex()
                .w_full()
                .gap_4()
                .child(
                    h_flex().w_full().justify_center().child(
                        h_flex()
                            .w_full()
                            .max_w(px(576.0))
                            .child(search_filter_input(
                                &self.panel_forms.forwarding.filter_input,
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
                            let entity = entity.clone();
                            move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.handle_forward_view_mode_change(ProfileViewMode::Grid, cx);
                                });
                            }
                        }))
                        .child(page_view_mode_toolbar_item(AppIcon::List, is_list, {
                            let entity = entity.clone();
                            move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.handle_forward_view_mode_change(ProfileViewMode::List, cx);
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
                    .child(
                        div().flex_1().min_w(px(0.0)).min_h(px(0.0)).child(
                            div().size_full().overflow_y_scrollbar().child(
                                v_flex()
                                    .w_full()
                                    .pb_8()
                                    .child(self.render_port_forward_saved_rules(entity, cx)),
                            ),
                        ),
                    ),
            )
            .into_any_element()
    }
}
