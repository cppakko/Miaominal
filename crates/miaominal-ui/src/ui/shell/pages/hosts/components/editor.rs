use crate::ui::components::editor_button;
use crate::ui::{components::SectionCard, i18n};

use super::super::super::super::*;
use gpui_component::{
    Size,
    tab::{Tab, TabBar},
};

#[path = "editor/fields.rs"]
mod fields;
#[path = "editor/sections.rs"]
mod sections;

use fields::{editor_environment_variable_row, editor_static_field};
use sections::proxy_jump_stepper_item;

impl AppView {
    pub(in crate::ui::shell) fn render_hosts_editor_sidebar(
        &self,
        entity: Entity<Self>,
        cx: &App,
    ) -> impl IntoElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let host_editor = &self.host_editor_forms;
        let available_groups = self.available_groups();
        let auth_method = host_editor.editing_auth_method;

        let title = if self.editors.host_editor_is_new {
            i18n::string("hosts.editor.titles.add")
        } else {
            i18n::string("hosts.editor.titles.edit")
        };
        let show_delete = !self.editors.host_editor_is_new && self.data.selected_profile.is_some();

        let current_profile_id = self.current_host_editor_profile_id().unwrap_or("");
        let proxy_jump_chain_profiles = self.proxy_jump_chain_profiles();
        let available_proxy_jump_candidates =
            self.available_proxy_jump_candidates(current_profile_id);
        let has_proxy_jump_candidates = !available_proxy_jump_candidates.is_empty();
        let selected_proxy_jump_step = host_editor
            .selected_proxy_jump_hop
            .filter(|index| *index < proxy_jump_chain_profiles.len())
            .unwrap_or(proxy_jump_chain_profiles.len());

        let target_name = host_editor.name_input.read(cx).value().trim().to_string();
        let target_step_title = if target_name.is_empty() {
            SharedString::from(i18n::string("hosts.editor.proxy_jump.target"))
        } else {
            SharedString::from(target_name.clone())
        };

        let auth_method_selected_index = match auth_method {
            AuthMethod::Password => 0,
            AuthMethod::KeyFile | AuthMethod::ManagedKey => 1,
            AuthMethod::Agent => 2,
            AuthMethod::KeyboardInteractive => 3,
        };
        let auth_method_tabs = TabBar::new("host-editor-auth-method")
            .w_full()
            .pill()
            .with_size(Size::Small)
            .selected_index(auth_method_selected_index)
            .on_click({
                let entity = entity.clone();
                move |index, _, cx| {
                    let auth_method = match *index {
                        0 => AuthMethod::Password,
                        1 => AuthMethod::ManagedKey,
                        2 => AuthMethod::Agent,
                        3 => AuthMethod::KeyboardInteractive,
                        _ => return,
                    };
                    entity.update(cx, |this, cx| {
                        this.set_auth_method(auth_method, cx);
                    });
                }
            })
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("hosts.editor.auth_methods.password")),
            )
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("hosts.editor.auth_methods.managed_key")),
            )
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("hosts.editor.auth_methods.ssh_agent")),
            )
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("hosts.editor.auth_methods.interactive")),
            );
        let agent_forwarding_tabs = TabBar::new("host-editor-agent-forwarding")
            .w_full()
            .pill()
            .with_size(Size::Small)
            .selected_index(if host_editor.agent_forwarding_enabled {
                1
            } else {
                0
            })
            .on_click({
                let entity = entity.clone();
                move |index, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.set_agent_forwarding_enabled(*index == 1, cx);
                    });
                }
            })
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("settings.values.off")),
            )
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("settings.values.on")),
            );

        let shell_type_selected_index = match host_editor.shell_type {
            ShellType::Posix => 0,
            ShellType::Fish => 1,
            ShellType::PowerShell => 2,
            ShellType::Cmd => 3,
        };
        let shell_type_tabs = TabBar::new("host-editor-shell-type")
            .w_full()
            .pill()
            .with_size(Size::Small)
            .selected_index(shell_type_selected_index)
            .on_click({
                let entity = entity.clone();
                move |index, _, cx| {
                    let shell_type = match *index {
                        0 => ShellType::Posix,
                        1 => ShellType::Fish,
                        2 => ShellType::PowerShell,
                        3 => ShellType::Cmd,
                        _ => return,
                    };
                    entity.update(cx, |this, cx| {
                        this.set_shell_type(shell_type, cx);
                    });
                }
            })
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("hosts.editor.shell_types.posix")),
            )
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("hosts.editor.shell_types.fish")),
            )
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("hosts.editor.shell_types.powershell")),
            )
            .child(
                Tab::new()
                    .flex_1()
                    .label(i18n::string("hosts.editor.shell_types.cmd")),
            );

        let mut proxy_jump_items = Vec::new();
        for (index, profile) in proxy_jump_chain_profiles.iter().enumerate() {
            let profile_label = if profile.name.trim().is_empty() {
                profile.summary()
            } else {
                profile.name.clone()
            };
            let hop = (index + 1).to_string();
            proxy_jump_items.push(proxy_jump_stepper_item(
                IconName::Building2,
                truncate_with_ellipsis(&profile_label, 24),
                SharedString::from(i18n::string_args(
                    "hosts.editor.proxy_jump.hop",
                    &[("index", &hop)],
                )),
            ));
        }
        proxy_jump_items.push(proxy_jump_stepper_item(
            IconName::CircleCheck,
            truncate_with_ellipsis(target_step_title.as_ref(), 24),
            SharedString::from(i18n::string("hosts.editor.proxy_jump.target")),
        ));

        let can_remove_environment_variable = host_editor.environment_variable_rows.len() > 1;
        let mut environment_variables = v_flex().w_full().gap_3().items_center();
        for (index, variable) in host_editor.environment_variable_rows.iter().enumerate() {
            let remove_entity = entity.clone();
            environment_variables = environment_variables.child(editor_environment_variable_row(
                index,
                variable.name_input.clone(),
                variable.value_input.clone(),
                can_remove_environment_variable,
                move |window, cx| {
                    remove_entity.update(cx, |this, cx| {
                        this.remove_environment_variable_row(index, window, cx);
                    });
                },
            ));
        }
        environment_variables = environment_variables.child(icon_button(
            AppIcon::Plus,
            30.0,
            99.0,
            None,
            None,
            Some(roles.outline_variant),
            {
                let entity = entity.clone();
                move |window, cx| {
                    entity.update(cx, |this, cx| {
                        this.add_environment_variable_row(window, cx);
                    });
                }
            },
        ));

        let header = h_flex().w_full().items_start().gap_4().child(
            v_flex().flex_1().gap_1().child(
                div()
                    .text_size(miaominal_settings::FontSize::PageTitle.scaled())
                    .text_color(rgb(roles.on_surface))
                    .child(title),
            ),
        );

        let general_section = SectionCard::new(
            AppIcon::Notebook,
            i18n::string("hosts.editor.sections.general"),
            v_flex()
                .gap_3()
                .child(surface_text_input_stack(
                    i18n::string("hosts.editor.fields.address"),
                    host_editor.host_input.clone(),
                    TextInputSurface::Low,
                    false,
                ))
                .child(surface_text_input_stack(
                    i18n::string("hosts.editor.fields.ssh_port"),
                    host_editor.port_input.clone(),
                    TextInputSurface::Low,
                    false,
                ))
                .child(surface_text_input_stack(
                    i18n::string("hosts.editor.fields.label"),
                    host_editor.name_input.clone(),
                    TextInputSurface::Low,
                    false,
                ))
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(i18n::string("hosts.editor.fields.group")),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap_2()
                                .child(
                                    md3_select(&host_editor.group_select)
                                        .large()
                                        .w_full()
                                        .rounded(px(14.0))
                                        .border_0()
                                        .bg(rgb(roles.surface_container_low))
                                        .cleanable(true)
                                        .placeholder(if available_groups.is_empty() {
                                            i18n::string("hosts.editor.group.no_existing_groups")
                                        } else {
                                            i18n::string("hosts.editor.group.select_existing")
                                        })
                                        .disabled(available_groups.is_empty()),
                                )
                                .child(icon_button(
                                    AppIcon::Plus,
                                    30.0,
                                    99.0,
                                    None,
                                    None,
                                    Some(roles.outline_variant),
                                    {
                                        let entity = entity.clone();
                                        move |window, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.begin_new_group(window, cx);
                                            });
                                        }
                                    },
                                )),
                        )
                        .when(host_editor.creating_new_group, |this| {
                            this.child(
                                surface_text_input(&host_editor.group_input, TextInputSurface::Low)
                                    .large(),
                            )
                        }),
                )
                .child(surface_text_input_stack(
                    i18n::string("hosts.editor.fields.tags"),
                    host_editor.tags_input.clone(),
                    TextInputSurface::Low,
                    false,
                ))
                .child(editor_static_field(
                    i18n::string("hosts.editor.fields.backspace"),
                    i18n::string("hosts.editor.values.default"),
                )),
        );

        let credentials_section = SectionCard::new(
            AppIcon::Key,
            i18n::string("hosts.editor.sections.credentials"),
            v_flex()
                .gap_3()
                .child(surface_text_input_stack(
                    i18n::string("hosts.editor.fields.username"),
                    host_editor.username_input.clone(),
                    TextInputSurface::Low,
                    false,
                ))
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(i18n::string("hosts.editor.fields.identity_source")),
                        )
                        .child(auth_method_tabs),
                )
                .when(auth_method == AuthMethod::Password, |this| {
                    this.child(surface_secret_text_input_stack(
                        i18n::string("hosts.editor.fields.password"),
                        host_editor.password_input.clone(),
                        crate::ui::components::SecretTextInputStackOptions {
                            surface: TextInputSurface::Low,
                            size: Size::Large,
                            required: false,
                            disabled: false,
                            reveal_icon: self.secret_reveal_icon(SecretRevealTarget::HostPassword),
                        },
                        {
                            let entity = entity.clone();
                            move |window, cx| {
                                entity.update(cx, |this, cx| {
                                    this.toggle_secret_visibility(
                                        SecretRevealTarget::HostPassword,
                                        window,
                                        cx,
                                    );
                                });
                            }
                        },
                    ))
                })
                .when(
                    auth_method == AuthMethod::ManagedKey || auth_method == AuthMethod::KeyFile,
                    |this| {
                        this.child(
                            v_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(miaominal_settings::FontSize::Body.scaled())
                                        .text_color(rgb(roles.on_surface_variant))
                                        .child(i18n::string("hosts.editor.fields.managed_key_id")),
                                )
                                .child(
                                    md3_select(&host_editor.managed_key_select)
                                        .large()
                                        .w_full()
                                        .rounded(px(14.0))
                                        .border_0()
                                        .bg(rgb(roles.surface_container_low))
                                        .cleanable(true)
                                        .placeholder(i18n::string(
                                            "placeholders.host_editor.managed_key_id",
                                        )),
                                ),
                        )
                    },
                )
                .when(auth_method == AuthMethod::Agent, |this| {
                    this.child(surface_text_input_stack(
                        i18n::string("hosts.editor.fields.agent_identity"),
                        host_editor.agent_identity_input.clone(),
                        TextInputSurface::Low,
                        false,
                    ))
                })
                .when(auth_method != AuthMethod::Password, |this| {
                    this.child(surface_text_input_stack(
                        i18n::string("hosts.editor.fields.certificate"),
                        host_editor.certificate_input.clone(),
                        TextInputSurface::Low,
                        false,
                    ))
                }),
        );

        let advanced_section = SectionCard::new(
            AppIcon::Settings,
            i18n::string("hosts.editor.sections.advanced"),
            v_flex()
                .gap_3()
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(i18n::string("hosts.editor.fields.agent_forwarding")),
                        )
                        .child(agent_forwarding_tabs),
                )
                .child(surface_text_editor_stack(
                    i18n::string("hosts.editor.fields.startup_command"),
                    host_editor.startup_command_input.clone(),
                    116.0,
                    TextInputSurface::Low,
                    false,
                ))
                .child(
                    v_flex()
                        .w_full()
                        .gap_3()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(i18n::string("hosts.editor.fields.host_chaining")),
                        )
                        .child(
                            v_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(miaominal_settings::FontSize::Body.scaled())
                                        .text_color(rgb(roles.on_surface_variant))
                                        .child(i18n::string("hosts.editor.proxy_jump.add_jump_host")),
                                )
                                .when(
                                    proxy_jump_chain_profiles.len() == self.data.sessions.len()
                                        || !has_proxy_jump_candidates,
                                    |this| {
                                        this.child(editor_static_field(
                                            i18n::string("hosts.editor.proxy_jump.available_hosts"),
                                            i18n::string(
                                                "hosts.editor.proxy_jump.no_additional_saved_hosts",
                                            ),
                                        ))
                                    },
                                )
                                .when(
                                    has_proxy_jump_candidates,
                                    |this| {
                                        this.child(
                                            v_flex()
                                                .w_full()
                                                .gap_2()
                                                .child(
                                                    h_flex()
                                                        .w_full()
                                                        .items_center()
                                                        .gap_2()
                                                        .child(
                                                            md3_select(&host_editor.proxy_jump_select)
                                                                .large()
                                                                .w_full()
                                                                .rounded(px(14.0))
                                                                .border_0()
                                                                .bg(rgb(roles.surface_container_low))
                                                                .cleanable(true)
                                                                .placeholder(i18n::string(
                                                                    "hosts.editor.proxy_jump.search_saved_hosts",
                                                                ))
                                                                .disabled(
                                                                    !has_proxy_jump_candidates,
                                                                ),
                                                        ),
                                                ),
                                        )
                                    },
                                ),
                        )
                        .child(
                            div()
                                .w_full()
                                .rounded(px(14.0))
                                .bg(rgb(roles.surface_container_low))
                                .p_3()
                                .child(
                                    v_flex()
                                        .w_full()
                                        .gap_3()
                                        .child(
                                            Stepper::new("proxy-jump-stepper")
                                                .vertical()
                                                .selected_index(selected_proxy_jump_step)
                                                .items(proxy_jump_items)
                                                .on_click({
                                                    let entity = entity.clone();
                                                    move |step, _, cx| {
                                                        entity.update(cx, |this, cx| {
                                                            this.select_proxy_jump_step(*step, cx);
                                                        });
                                                    }
                                                }),
                                        )
                                        .child(
                                            v_flex()
                                                .w_full()
                                                .gap_2()
                                                .child(
                                                    h_flex()
                                                        .gap_2()
                                                        .flex_wrap()
                                                        .child(icon_button(
                                                            AppIcon::Upload,
                                                            30.0,
                                                            99.0,
                                                            Some(roles.surface_container_highest),
                                                            Some(roles.on_surface),
                                                            Some(roles.outline_variant),
                                                            {
                                                                let entity = entity.clone();
                                                                move |_, cx| {
                                                                    entity.update(cx, |this, cx| {
                                                                        this.move_selected_proxy_jump_hop_up(cx);
                                                                    });
                                                                }
                                                            },
                                                        ))
                                                        .child(icon_button(
                                                            AppIcon::Download,
                                                            30.0,
                                                            99.0,
                                                            Some(roles.surface_container_highest),
                                                            Some(roles.on_surface),
                                                            Some(roles.outline_variant),
                                                            {
                                                                let entity = entity.clone();
                                                                move |_, cx| {
                                                                    entity.update(cx, |this, cx| {
                                                                        this.move_selected_proxy_jump_hop_down(cx);
                                                                    });
                                                                }
                                                            },
                                                        ))
                                                        .child(icon_button(
                                                            AppIcon::Close,
                                                            30.0,
                                                            99.0,
                                                            Some(roles.surface_container_highest),
                                                            Some(roles.on_surface),
                                                            Some(roles.outline_variant),
                                                            {
                                                                let entity = entity.clone();
                                                                move |_, cx| {
                                                                    entity.update(cx, |this, cx| {
                                                                        this.remove_selected_proxy_jump_hop(cx);
                                                                    });
                                                                }
                                                            },
                                                        )),
                                                ),
                                        ),
                                ),
                        )
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_3()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                            .child(i18n::string("hosts.editor.environment_variables.section_title")),
                        )
                        .child(environment_variables),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(i18n::string("hosts.editor.fields.shell_type")),
                        )
                        .child(shell_type_tabs),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(i18n::string("hosts.editor.fields.character_set")),
                        )
                        .child(
                            md3_select(&host_editor.charset_select)
                                .large()
                                .w_full()
                                .rounded(px(14.0))
                                .border_0()
                                .bg(rgb(roles.surface_container_low))
                                .placeholder(i18n::string(
                                    "hosts.editor.fields.search_character_set",
                                )),
                        ),
                ),
        );

        let mut footer_actions = vec![
            editor_button(
                i18n::string("hosts.editor.buttons.test_connection"),
                false,
                true,
                {
                    let entity = entity.clone();
                    move |window, cx| {
                        entity.update(cx, |this, cx| this.test_profile_connection(window, cx));
                    }
                },
            )
            .into_any_element(),
        ];
        if show_delete {
            footer_actions.push(
                icon_button(
                    AppIcon::Trash,
                    36.0,
                    12.0,
                    Some(roles.error_container),
                    Some(roles.on_error_container),
                    None,
                    {
                        let entity = entity.clone();
                        move |window, cx| {
                            entity.update(cx, |this, cx| this.delete_selected_profile(window, cx));
                        }
                    },
                )
                .into_any_element(),
            );
        }
        footer_actions.push(
            icon_button(AppIcon::Close, 36.0, 12.0, None, None, None, {
                let entity = entity.clone();
                move |_, cx| {
                    entity.update(cx, |this, cx| this.close_host_editor(cx));
                }
            })
            .into_any_element(),
        );
        footer_actions.push(
            icon_button(
                AppIcon::Check,
                36.0,
                12.0,
                Some(roles.primary),
                Some(roles.on_primary),
                None,
                {
                    let entity = entity.clone();
                    move |window, cx| {
                        entity.update(cx, |this, cx| this.save_profile(window, cx));
                    }
                },
            )
            .into_any_element(),
        );
        let footer = editor_footer_actions(footer_actions).h_full();

        div()
            .id("host-editor-sidebar")
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
                                    .gap_3()
                                    .pb_4()
                                    .child(general_section)
                                    .child(credentials_section)
                                    .child(advanced_section),
                            ),
                        ),
                    )
                    .child(div().px_4().py_4().child(footer)),
            )
    }
}
