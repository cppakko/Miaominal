use super::*;
use crate::ui::components::card_grid;
use crate::ui::components::catalog_card::{
    catalog_card_chip, catalog_card_chips, catalog_card_content, catalog_card_icon,
    catalog_card_surface, catalog_card_title,
};
use crate::ui::components::empty_state::empty_state as shell_empty_state;
use crate::ui::components::shell_tokens::{ShellMetrics, space};
use crate::ui::components::{EditorFooterActionTone, SegmentedSwitch, editor_footer_button};
use crate::ui::{
    components::{SectionCard, md3_spinner},
    i18n,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu};
use miaominal_core::keychain::{ManagedKeyGenerationAlgorithm, ManagedKeySource};

fn keychain_empty_state(
    title: impl Into<SharedString>,
    copy: impl Into<SharedString>,
) -> impl IntoElement {
    let title = title.into();
    let copy = copy.into();

    shell_empty_state(AppIcon::Key, format!("{}\n{}", title, copy))
}

fn managed_key_source_label(source: ManagedKeySource) -> String {
    match source {
        ManagedKeySource::Generated => i18n::string("keychain.source.generated"),
        ManagedKeySource::Imported => i18n::string("keychain.source.imported"),
    }
}

fn build_managed_key_context_menu(
    menu: PopupMenu,
    entity: Entity<KeychainController>,
    rename_id: String,
    delete_id: String,
) -> PopupMenu {
    let rename_entity = entity.clone();
    let delete_entity = entity;

    menu.item(
        PopupMenuItem::new(i18n::string("keychain.menu.rename")).on_click(move |_, window, cx| {
            rename_entity.update(cx, |this, cx| {
                this.request_managed_key_rename(&rename_id, window, cx);
            });
        }),
    )
    .item(
        PopupMenuItem::new(i18n::string("keychain.menu.delete")).on_click(move |_, _, cx| {
            delete_entity.update(cx, |this, cx| {
                this.request_managed_key_delete(&delete_id, cx);
            });
        }),
    )
}

fn managed_key_card(
    key: &ManagedKeyRecord,
    entity: Entity<KeychainController>,
) -> impl IntoElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let source_label = managed_key_source_label(key.source);
    let deploy_id = key.id.clone();
    let rename_id = key.id.clone();
    let delete_id = key.id.clone();
    let item_id = SharedString::from(format!("managed-key-card-{}", key.id.as_str()));
    let context_entity = entity.clone();

    catalog_card_surface()
        .id(item_id)
        .test_support()
        .context_menu(move |menu, _window, _cx| {
            build_managed_key_context_menu(
                menu,
                context_entity.clone(),
                rename_id.clone(),
                delete_id.clone(),
            )
        })
        .child(catalog_card_content(
            catalog_card_icon(AppIcon::Key),
            catalog_card_title(key.name.clone()),
            Some(key.algorithm.clone().into()),
            Some(
                icon_button(
                    AppIcon::Upload,
                    30.0,
                    10.0,
                    Some(roles.primary_container),
                    Some(roles.on_primary_container),
                    Some(roles.primary),
                    {
                        let entity = entity.clone();
                        move |window, cx| {
                            entity.update(cx, |this, cx| {
                                this.open_keychain_deploy_editor(&deploy_id, window, cx);
                            });
                        }
                    },
                )
                .into_any_element(),
            ),
            Some(
                catalog_card_chips()
                    .child(catalog_card_chip(source_label, None))
                    .child(catalog_card_chip(
                        i18n::string("keychain.card.os_keyring_badge"),
                        None,
                    ))
                    .into_any_element(),
            ),
        ))
}

fn agent_identity_card(identity: &miaominal_ssh::AgentIdentitySummary) -> impl IntoElement {
    let comment = if identity.comment.trim().is_empty() {
        i18n::string("keychain.card.no_comment")
    } else {
        identity.comment.clone()
    };
    let item_id = SharedString::from(format!(
        "agent-identity-card-{}",
        identity.serialized.as_str()
    ));

    catalog_card_surface()
        .id(item_id)
        .test_support()
        .child(catalog_card_content(
            catalog_card_icon(AppIcon::FingerPrint),
            catalog_card_title(identity.label.clone()),
            Some(comment.into()),
            None,
            Some(
                catalog_card_chips()
                    .child(catalog_card_chip(identity.kind.clone(), None))
                    .child(catalog_card_chip(
                        i18n::string("keychain.card.available_through_ssh_agent"),
                        None,
                    ))
                    .into_any_element(),
            ),
        ))
}

impl KeychainController {
    pub(in crate::ui) fn render_keychain_page(
        &self,
        entity: Entity<Self>,
        available_width: f32,
        _cx: &App,
    ) -> gpui_kit::AnyElement {
        let keychain_page_view = self.page_view;
        let selected_keychain_page_view_index = match keychain_page_view {
            KeychainPageView::ManagedKeys => 0,
            KeychainPageView::AgentIdentities => 1,
        };
        let show_managed_keys = keychain_page_view == KeychainPageView::ManagedKeys;
        let show_agent_identities = keychain_page_view == KeychainPageView::AgentIdentities;

        let mut visible_managed_keys: Vec<_> = self.managed_keys.iter().collect();
        visible_managed_keys.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut visible_agent_identities: Vec<_> = self.agent_identities.iter().collect();
        visible_agent_identities.sort_by(|left, right| {
            left.label
                .to_ascii_lowercase()
                .cmp(&right.label.to_ascii_lowercase())
                .then_with(|| left.serialized.cmp(&right.serialized))
        });

        let header = v_flex()
            .w_full()
            .gap_6()
            .px(space(ShellMetrics::CATALOG_INSET))
            .child(
                h_flex().w_full().justify_center().child(
                    SegmentedSwitch::new("keychain-view-tabs")
                        .selected_index(selected_keychain_page_view_index)
                        .width(332.0_f32.min(
                            miaominal_settings::scaled_font_size(available_width.max(1.0)).as_f32(),
                        ))
                        .height(34.0)
                        .padding(2.0)
                        .item(i18n::string("keychain.page.managed_keys"))
                        .item(i18n::string("keychain.page.agent_identities"))
                        .on_click({
                            let entity = entity.clone();
                            move |index, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.page_view = match index {
                                        0 => KeychainPageView::ManagedKeys,
                                        _ => KeychainPageView::AgentIdentities,
                                    };
                                    cx.notify();
                                });
                            }
                        }),
                ),
            );

        let content = v_flex()
            .w_full()
            .min_w_0()
            .max_w(space(ShellMetrics::CONTENT_MAX_WIDTH))
            .mx_auto()
            .gap_6()
            .pb(space(112.0))
            .when(show_managed_keys, |this| {
                this.child(
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
                                    "keychain.page.managed_keys",
                                ))),
                        )
                        .child(if visible_managed_keys.is_empty() {
                            keychain_empty_state(
                                i18n::string("keychain.empty.no_managed_keys_title"),
                                i18n::string("keychain.empty.no_managed_keys_copy"),
                            )
                            .into_any_element()
                        } else {
                            card_grid(available_width)
                                .id("keychain-managed-grid")
                                .test_support()
                                .children(visible_managed_keys.into_iter().map(|key| {
                                    managed_key_card(key, entity.clone()).into_any_element()
                                }))
                                .into_any_element()
                        }),
                )
            })
            .when(show_agent_identities, |this| {
                this.child(
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
                                    "keychain.page.agent_identities",
                                ))),
                        )
                        .child(if visible_agent_identities.is_empty() {
                            keychain_empty_state(
                                i18n::string("keychain.empty.no_agent_identities_title"),
                                i18n::string("keychain.empty.no_agent_identities_copy"),
                            )
                            .into_any_element()
                        } else {
                            card_grid(available_width)
                                .id("keychain-agent-grid")
                                .test_support()
                                .children(visible_agent_identities.into_iter().map(|identity| {
                                    agent_identity_card(identity).into_any_element()
                                }))
                                .into_any_element()
                        }),
                )
            });

        div()
            .size_full()
            .child(
                v_flex().size_full().gap_6().child(header).child(
                    div().flex_1().w_full().min_h_0().child(
                        div()
                            .size_full()
                            .overflow_y_scrollbar()
                            .child(div().px(space(ShellMetrics::CATALOG_INSET)).child(content)),
                    ),
                ),
            )
            .into_any_element()
    }

    /// Refresh and create commands, docked into the bottom navigation bar.
    pub(in crate::ui) fn render_keychain_dock_action(
        &self,
        entity: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        h_flex()
            .gap(space(8.0))
            .child(dock_icon_action_button(
                "keychain-refresh-fab",
                AppIcon::Rotate,
                i18n::string("keychain.page.refresh"),
                false,
                cx,
                {
                    let entity = entity.clone();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.refresh_keychain_data(cx);
                        });
                    }
                },
            ))
            .child(dock_action_button(
                "keychain-create-fab",
                i18n::string("keychain.editor.title"),
                cx,
                move |window, cx| {
                    entity.update(cx, |this, cx| this.open_keychain_editor(window, cx));
                },
            ))
            .into_any_element()
    }

    pub(in crate::ui) fn render_keychain_editor_sidebar(
        &self,
        entity: Entity<Self>,
        cx: &App,
    ) -> impl IntoElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let store_available = self.keychain_store.is_some();
        let generation_in_progress = self.generation_in_progress;
        let import_available = store_available && !generation_in_progress;
        let forms = &self.forms;
        let is_deploy_mode = self.editor_mode == KeychainEditorMode::Deploy;
        let deploy_in_progress = self.deploy_in_progress;
        let selected_deploy_key = self.keychain_selected_deploy_key();
        let deployable_profile_count = self
            .catalog
            .profiles()
            .iter()
            .filter(|profile| Self::keychain_profile_supports_deploy(profile))
            .count();
        let header = h_flex()
            .w_full()
            .items_start()
            .justify_between()
            .gap_3()
            .child(
                v_flex().flex_1().gap_1().child(
                    div()
                        .text_size(miaominal_settings::FontSize::PageTitle.scaled())
                        .text_color(rgb(roles.on_surface))
                        .child(if is_deploy_mode {
                            i18n::string("keychain.deploy.title")
                        } else {
                            i18n::string("keychain.editor.title")
                        }),
                ),
            )
            .child(badge(
                if is_deploy_mode {
                    i18n::string("keychain.deploy.badge")
                } else if store_available {
                    i18n::string("keychain.editor.os_keyring_ready")
                } else {
                    i18n::string("keychain.editor.storage_unavailable")
                },
                if is_deploy_mode {
                    roles.primary_container
                } else if store_available {
                    roles.surface_container_high
                } else {
                    roles.surface_container_highest
                },
                if is_deploy_mode {
                    roles.on_primary_container
                } else if store_available {
                    roles.on_surface_variant
                } else {
                    roles.error
                },
            ));

        let key_details_section = SectionCard::new(
            AppIcon::Key,
            i18n::string("keychain.editor.details"),
            v_flex()
                .gap_3()
                .child(surface_text_input_stack(
                    i18n::string("keychain.editor.key_name"),
                    forms.name_input.clone(),
                    TextInputSurface::Low,
                    false,
                ))
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(field_label(
                            i18n::string("keychain.editor.import_private_key_file"),
                            false,
                        ))
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap_2()
                                .child(
                                    div().flex_1().min_w(px(0.0)).child(
                                        surface_text_input(
                                            &forms.import_path_input,
                                            TextInputSurface::Low,
                                        )
                                        .large(),
                                    ),
                                )
                                .child(icon_button(
                                    AppIcon::Folder,
                                    30.0,
                                    10.0,
                                    None,
                                    None,
                                    Some(roles.outline_variant),
                                    {
                                        let entity = entity.clone();
                                        move |window, cx| {
                                            if !store_available {
                                                return;
                                            }

                                            entity.update(cx, |this, cx| {
                                                this.open_import_file_picker(window, cx);
                                            });
                                        }
                                    },
                                )),
                        )
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .line_height(miaominal_settings::scaled_line_height(18.0))
                                .text_color(rgb(text_muted))
                                .child(i18n::string(
                                    "keychain.editor.choose_private_key_file_copy",
                                )),
                        ),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(field_label(
                            i18n::string("keychain.editor.or_paste_private_key"),
                            false,
                        ))
                        .child(surface_text_editor(
                            &forms.import_private_key_input,
                            188.0,
                            TextInputSurface::Low,
                        )),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(field_label(
                            i18n::string("keychain.editor.or_paste_public_key"),
                            false,
                        ))
                        .child(surface_text_editor(
                            &forms.import_public_key_input,
                            188.0,
                            TextInputSurface::Low,
                        )),
                )
                .child(surface_text_input_stack(
                    i18n::string("keychain.editor.import_passphrase"),
                    forms.import_passphrase_input.clone(),
                    TextInputSurface::Low,
                    false,
                )),
        );

        let deploy_profile_select = md3_select(&forms.deploy_profile_select)
            .large()
            .w_full()
            .rounded(px(14.0))
            .border_0()
            .bg(rgb(roles.surface_container_low))
            .icon(IconName::Search)
            .cleanable(true)
            .search_placeholder(i18n::string("keychain.deploy.search_profiles"))
            .placeholder(if deployable_profile_count == 0 {
                i18n::string("keychain.deploy.no_profiles")
            } else {
                i18n::string("keychain.deploy.select_profile")
            });

        let deploy_section = SectionCard::new(
            AppIcon::Upload,
            i18n::string("keychain.deploy.section"),
            v_flex()
                .gap_4()
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(field_label(
                            i18n::string("keychain.deploy.selected_key"),
                            false,
                        ))
                        .child(
                            v_flex()
                                .w_full()
                                .gap_2()
                                .p_3()
                                .rounded(px(16.0))
                                .bg(rgb(roles.surface_container_low))
                                .when_some(selected_deploy_key, |this, key| {
                                    this.child(
                                        div()
                                            .text_size(
                                                miaominal_settings::FontSize::Heading.scaled(),
                                            )
                                            .text_color(rgb(roles.on_surface))
                                            .child(key.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(miaominal_settings::FontSize::Body.scaled())
                                            .text_color(rgb(text_muted))
                                            .child(key.summary()),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .flex_wrap()
                                            .child(badge(
                                                managed_key_source_label(key.source),
                                                roles.surface_container_high,
                                                roles.on_surface_variant,
                                            ))
                                            .child(badge(
                                                key.algorithm.clone(),
                                                roles.surface_container_high,
                                                roles.on_surface_variant,
                                            )),
                                    )
                                })
                                .when(selected_deploy_key.is_none(), |this| {
                                    this.child(
                                        div()
                                            .text_size(miaominal_settings::FontSize::Input.scaled())
                                            .text_color(rgb(text_muted))
                                            .child(i18n::string("keychain.deploy.key_missing")),
                                    )
                                }),
                        ),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(field_label(i18n::string("keychain.deploy.profile"), true))
                        .child(deploy_profile_select)
                        .when(deployable_profile_count == 0, |this| {
                            this.child(
                                div()
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .line_height(miaominal_settings::scaled_line_height(18.0))
                                    .text_color(rgb(text_muted))
                                    .child(i18n::string("keychain.deploy.no_profiles_copy")),
                            )
                        }),
                )
                .child(surface_text_input_stack(
                    i18n::string("keychain.deploy.location"),
                    forms.deploy_location_input.clone(),
                    TextInputSurface::Low,
                    true,
                ))
                .child(surface_text_input_stack(
                    i18n::string("keychain.deploy.filename"),
                    forms.deploy_filename_input.clone(),
                    TextInputSurface::Low,
                    true,
                ))
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(i18n::string("keychain.deploy.command")),
                        )
                        .child(surface_text_editor(
                            &forms.deploy_command_input,
                            176.0,
                            TextInputSurface::Low,
                        ))
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .line_height(miaominal_settings::scaled_line_height(18.0))
                                .text_color(rgb(text_muted))
                                .child(i18n::string("keychain.deploy.command_copy")),
                        ),
                ),
        );

        let generate_disabled = !store_available || generation_in_progress;
        let generate_button = editor_footer_button(
            "keychain-editor-footer-generate",
            i18n::string(if generation_in_progress {
                "keychain.editor.generating"
            } else {
                "keychain.editor.generate_key"
            }),
            None,
            EditorFooterActionTone::Text,
            cx,
            |_, _| {},
        )
        .disabled(generate_disabled)
        .opacity(if generate_disabled { 0.38 } else { 1.0 })
        .child(Icon::new(IconName::ChevronDown).xsmall())
        .dropdown_menu({
            let menu_entity = entity.clone();
            move |menu, _, _| {
                let ed25519_entity = menu_entity.clone();
                let rsa_entity = menu_entity.clone();
                menu.min_w(220.0)
                    .item(
                        PopupMenuItem::new(i18n::string("keychain.editor.generate_ed25519"))
                            .on_click(move |_, window, cx| {
                                ed25519_entity.update(cx, |this, cx| {
                                    this.generate_managed_key(
                                        ManagedKeyGenerationAlgorithm::Ed25519,
                                        window,
                                        cx,
                                    );
                                });
                            }),
                    )
                    .item(
                        PopupMenuItem::new(i18n::string("keychain.editor.generate_rsa_4096"))
                            .on_click(move |_, window, cx| {
                                rsa_entity.update(cx, |this, cx| {
                                    this.generate_managed_key(
                                        ManagedKeyGenerationAlgorithm::Rsa4096,
                                        window,
                                        cx,
                                    );
                                });
                            }),
                    )
            }
        });

        let import_footer = editor_footer_actions(vec![
            generate_button.into_any_element(),
            editor_footer_button(
                "keychain-editor-footer-cancel",
                i18n::string("dialogs.common.cancel"),
                Some(AppIcon::Close),
                EditorFooterActionTone::Text,
                cx,
                {
                    let entity = entity.clone();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.close_keychain_editor(cx);
                        });
                    }
                },
            )
            .into_any_element(),
            editor_footer_button(
                "keychain-editor-footer-import",
                i18n::string("keychain.editor.import_key"),
                Some(AppIcon::Upload),
                EditorFooterActionTone::Primary,
                cx,
                {
                    let entity = entity.clone();
                    move |window, cx| {
                        if !import_available {
                            return;
                        }

                        entity.update(cx, |this, cx| {
                            this.import_managed_key(window, cx);
                        });
                    }
                },
            )
            .disabled(!import_available)
            .opacity(if import_available { 1.0 } else { 0.38 })
            .into_any_element(),
        ]);

        let deploy_footer = editor_footer_actions(vec![
            editor_footer_button(
                "keychain-deploy-footer-cancel",
                i18n::string("dialogs.common.cancel"),
                Some(AppIcon::Close),
                EditorFooterActionTone::Text,
                cx,
                {
                    let entity = entity.clone();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.close_keychain_editor(cx);
                        });
                    }
                },
            )
            .into_any_element(),
            if deploy_in_progress {
                div()
                    .id("keychain-editor-footer-deploy-spinner")
                    .min_w(space(116.0))
                    .min_h(space(ShellMetrics::EDITOR_ACTION_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(md3_spinner(18.0))
                    .into_any_element()
            } else {
                let deploy_disabled =
                    selected_deploy_key.is_none() || deployable_profile_count == 0;
                editor_footer_button(
                    "keychain-editor-footer-deploy",
                    i18n::string("keychain.deploy.deploy_button"),
                    Some(AppIcon::Upload),
                    EditorFooterActionTone::Primary,
                    cx,
                    {
                        let entity = entity.clone();
                        move |window, cx| {
                            entity.update(cx, |this, cx| {
                                this.deploy_managed_key(window, cx);
                            });
                        }
                    },
                )
                .disabled(deploy_disabled)
                .opacity(if deploy_disabled { 0.38 } else { 1.0 })
                .into_any_element()
            },
        ]);

        div()
            .id("keychain-editor-sidebar")
            .w(px(EDITOR_DRAWER_WIDTH))
            .min_w(px(360.0))
            .h_full()
            .rounded(space(ShellMetrics::CARD_RADIUS))
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
                                    .child(if is_deploy_mode {
                                        deploy_section.into_any_element()
                                    } else {
                                        key_details_section.into_any_element()
                                    }),
                            ),
                        ),
                    )
                    .child(div().px_4().pt_3().pb_4().child(if is_deploy_mode {
                        deploy_footer.into_any_element()
                    } else {
                        import_footer.into_any_element()
                    })),
            )
    }
}
