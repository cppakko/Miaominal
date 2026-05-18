use super::super::super::*;
use super::super::empty_state::shell_empty_state;
use crate::domain::keychain::ManagedKeySource;
use crate::ui::components::editor_button_with_id;
use crate::ui::{
    components::{SectionCard, md3_spinner},
    i18n,
};
use gpui_component::tab::{Tab, TabBar};
use rfd::FileDialog;

const KEYCHAIN_CARD_WIDTH: f32 = 332.0;
const KEYCHAIN_CARD_ACTION_WIDTH: f32 = 52.0;

fn keychain_card_shell() -> Div {
    let roles = settings::current_theme().material.roles;

    card_surface(roles.surface_container, 20.0)
        .w(px(KEYCHAIN_CARD_WIDTH))
        .p_3()
}

fn keychain_empty_state(
    title: impl Into<SharedString>,
    copy: impl Into<SharedString>,
) -> impl IntoElement {
    let title = title.into();
    let copy = copy.into();

    shell_empty_state(AppIcon::Key, format!("{}\n{}", title, copy))
}

fn managed_key_matches_filter(key: &ManagedKeyRecord, filter_text: &str) -> bool {
    if filter_text.is_empty() {
        return true;
    }

    let source_label = managed_key_source_label(key.source).to_ascii_lowercase();

    format!(
        "{} {} {} {} {}",
        key.name, key.id, key.algorithm, key.public_key, source_label,
    )
    .to_ascii_lowercase()
    .contains(filter_text)
}

fn agent_identity_matches_filter(identity: &ssh::AgentIdentitySummary, filter_text: &str) -> bool {
    if filter_text.is_empty() {
        return true;
    }

    format!(
        "{} {} {} {}",
        identity.label, identity.comment, identity.kind, identity.serialized,
    )
    .to_ascii_lowercase()
    .contains(filter_text)
}

fn managed_key_source_label(source: ManagedKeySource) -> String {
    match source {
        ManagedKeySource::Generated => i18n::string("keychain.source.generated"),
        ManagedKeySource::Imported => i18n::string("keychain.source.imported"),
    }
}

fn managed_key_card(key: &ManagedKeyRecord, entity: Entity<AppView>) -> impl IntoElement {
    let material = settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let source_label = managed_key_source_label(key.source);
    let deploy_id = key.id.clone();
    let delete_id = key.id.clone();

    keychain_card_shell().child(
        h_flex()
            .w_full()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_3()
                    .child(
                        h_flex()
                            .w_full()
                            .items_start()
                            .gap_3()
                            .child(page_primary_icon_tile(AppIcon::Key, 40.0, 12.0))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(settings::scaled_font_size(15.0))
                                            .line_height(settings::scaled_line_height(20.0))
                                            .text_color(rgb(roles.on_surface))
                                            .child(key.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(settings::scaled_font_size(11.0))
                                            .text_color(rgb(text_muted))
                                            .child(key.algorithm.clone()),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(badge(
                                source_label,
                                roles.surface_container_high,
                                roles.on_surface_variant,
                            ))
                            .child(badge(
                                i18n::string("keychain.card.os_keyring_badge"),
                                roles.surface_container_high,
                                roles.on_surface_variant,
                            )),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(KEYCHAIN_CARD_ACTION_WIDTH))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        v_flex()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .child(icon_button(
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
                                            this.open_keychain_deploy_editor(
                                                &deploy_id, window, cx,
                                            );
                                        });
                                    }
                                },
                            ))
                            .child(icon_button(
                                AppIcon::Close,
                                30.0,
                                10.0,
                                None,
                                None,
                                Some(roles.outline_variant),
                                {
                                    let entity = entity.clone();
                                    move |_, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.request_managed_key_delete(&delete_id, cx);
                                        });
                                    }
                                },
                            )),
                    ),
            ),
    )
}

fn agent_identity_card(identity: &ssh::AgentIdentitySummary) -> impl IntoElement {
    let material = settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let comment = if identity.comment.trim().is_empty() {
        i18n::string("keychain.card.no_comment")
    } else {
        identity.comment.clone()
    };

    keychain_card_shell().child(
        h_flex().w_full().items_start().gap_3().child(
            v_flex()
                .flex_1()
                .min_w(px(0.0))
                .gap_3()
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .gap_3()
                        .child(page_primary_icon_tile(AppIcon::FingerPrint, 40.0, 12.0))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(settings::scaled_font_size(15.0))
                                        .line_height(settings::scaled_line_height(20.0))
                                        .text_color(rgb(roles.on_surface))
                                        .child(identity.label.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(settings::scaled_font_size(11.0))
                                        .text_color(rgb(text_muted))
                                        .child(comment),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(badge(
                            identity.kind.clone(),
                            roles.surface_container_high,
                            roles.on_surface_variant,
                        ))
                        .child(badge(
                            i18n::string("keychain.card.available_through_ssh_agent"),
                            roles.surface_container_high,
                            roles.on_surface_variant,
                        )),
                ),
        ),
    )
}

impl AppView {
    pub(in crate::ui::shell) fn render_keychain_page(
        &self,
        entity: Entity<Self>,
        cx: &App,
    ) -> gpui::AnyElement {
        let raw_filter_text = self
            .panel_forms
            .keychain
            .filter_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let filter_text = raw_filter_text.to_ascii_lowercase();
        let keychain_page_view = self.keychain_page_view;
        let selected_keychain_page_view_index = match keychain_page_view {
            KeychainPageView::ManagedKeys => 0,
            KeychainPageView::AgentIdentities => 1,
        };
        let show_managed_keys = keychain_page_view == KeychainPageView::ManagedKeys;
        let show_agent_identities = keychain_page_view == KeychainPageView::AgentIdentities;

        let mut visible_managed_keys: Vec<_> = self
            .data
            .managed_keys
            .iter()
            .filter(|key| managed_key_matches_filter(key, &filter_text))
            .collect();
        visible_managed_keys.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut visible_agent_identities: Vec<_> = self
            .data
            .agent_identities
            .iter()
            .filter(|identity| agent_identity_matches_filter(identity, &filter_text))
            .collect();
        visible_agent_identities.sort_by(|left, right| {
            left.label
                .to_ascii_lowercase()
                .cmp(&right.label.to_ascii_lowercase())
                .then_with(|| left.serialized.cmp(&right.serialized))
        });

        let header = v_flex()
            .w_full()
            .gap_6()
            .px_5()
            .child(
                h_flex()
                    .w_full()
                    .justify_center()
                    .child(
                        h_flex()
                            .w_full()
                            .max_w(px(576.0))
                            .child(search_filter_input(
                                &self.panel_forms.keychain.filter_input,
                                SearchInputStyle::Pill,
                                None,
                            )),
                    ),
            )
            .child(
                h_flex().w_full().justify_center().child(
                    TabBar::new("keychain-view-tabs")
                        .segmented()
                        .selected_index(selected_keychain_page_view_index)
                        .on_click({
                            let entity = entity.clone();
                            move |index, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.keychain_page_view = match *index {
                                        0 => KeychainPageView::ManagedKeys,
                                        _ => KeychainPageView::AgentIdentities,
                                    };
                                    cx.notify();
                                });
                            }
                        })
                        .child(Tab::new().label(i18n::string("keychain.page.managed_keys")))
                        .child(Tab::new().label(i18n::string("keychain.page.agent_identities"))),
                ),
            );

        let content = v_flex()
            .w_full()
            .gap_6()
            .px_5()
            .pb_8()
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
                            if self.data.managed_keys.is_empty() {
                                keychain_empty_state(
                                    i18n::string("keychain.empty.no_managed_keys_title"),
                                    i18n::string("keychain.empty.no_managed_keys_copy"),
                                )
                                .into_any_element()
                            } else {
                                keychain_empty_state(
                                    i18n::string("keychain.empty.no_matching_managed_keys_title"),
                                    i18n::string("keychain.empty.no_matching_managed_keys_copy"),
                                )
                                .into_any_element()
                            }
                        } else {
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_4()
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
                            if self.data.agent_identities.is_empty() {
                                keychain_empty_state(
                                    i18n::string("keychain.empty.no_agent_identities_title"),
                                    i18n::string("keychain.empty.no_agent_identities_copy"),
                                )
                                .into_any_element()
                            } else {
                                keychain_empty_state(
                                    i18n::string(
                                        "keychain.empty.no_matching_agent_identities_title",
                                    ),
                                    i18n::string(
                                        "keychain.empty.no_matching_agent_identities_copy",
                                    ),
                                )
                                .into_any_element()
                            }
                        } else {
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_4()
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
                    div()
                        .flex_1()
                        .w_full()
                        .min_h_0()
                        .child(div().size_full().overflow_y_scrollbar().child(content)),
                ),
            )
            .into_any_element()
    }

    pub(in crate::ui::shell) fn render_keychain_fab(
        &self,
        entity: Entity<Self>,
    ) -> impl IntoElement {
        h_flex()
            .gap_3()
            .child(fab_icon_button(AppIcon::Rotate, {
                let entity = entity.clone();
                move |_, cx| {
                    entity.update(cx, |this, cx| {
                        this.refresh_keychain_data(cx);
                    });
                }
            }))
            .child(fab_button(move |window, cx| {
                entity.update(cx, |this, cx| this.open_keychain_editor(window, cx));
            }))
    }

    pub(in crate::ui::shell) fn render_keychain_editor_sidebar(
        &self,
        entity: Entity<Self>,
    ) -> impl IntoElement {
        let material = settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let store_available = self.services.keychain_store.is_some();
        let forms = &self.panel_forms.keychain;
        let is_deploy_mode = self.keychain_editor_mode == KeychainEditorMode::Deploy;
        let deploy_in_progress = self.keychain_deploy_in_progress;
        let selected_deploy_key = self.keychain_selected_deploy_key();
        let deployable_profile_count = self
            .data
            .sessions
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
                        .text_size(settings::scaled_font_size(20.0))
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

                                            let Some(path) = FileDialog::new()
                                                .set_title(i18n::string(
                                                    "keychain.editor.select_private_key_to_import",
                                                ))
                                                .pick_file()
                                            else {
                                                return;
                                            };

                                            entity.update(cx, |this, cx| {
                                                this.set_managed_key_import_file_path(
                                                    path, window, cx,
                                                );
                                            });
                                        }
                                    },
                                )),
                        )
                        .child(
                            div()
                                .text_size(settings::scaled_font_size(11.0))
                                .line_height(settings::scaled_line_height(18.0))
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

        let deploy_profile_select = Select::new(&forms.deploy_profile_select)
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
                                            .text_size(settings::scaled_font_size(14.0))
                                            .text_color(rgb(roles.on_surface))
                                            .child(key.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(settings::scaled_font_size(11.0))
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
                                            .text_size(settings::scaled_font_size(12.0))
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
                                    .text_size(settings::scaled_font_size(11.0))
                                    .line_height(settings::scaled_line_height(18.0))
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
                                .text_size(settings::scaled_font_size(11.0))
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
                                .text_size(settings::scaled_font_size(11.0))
                                .line_height(settings::scaled_line_height(18.0))
                                .text_color(rgb(text_muted))
                                .child(i18n::string("keychain.deploy.command_copy")),
                        ),
                ),
        );

        let import_footer = editor_footer_actions(vec![
            editor_button_with_id(
                "keychain-editor-footer-generate",
                i18n::string("keychain.editor.generate_ed25519"),
                true,
                true,
                !store_available,
                {
                    let entity = entity.clone();
                    move |window, cx| {
                        entity.update(cx, |this, cx| {
                            this.generate_managed_key(window, cx);
                        });
                    }
                },
            )
            .into_any_element(),
            div()
                .id("keychain-editor-footer-cancel")
                .child(icon_button(
                    AppIcon::Close,
                    36.0,
                    12.0,
                    None,
                    None,
                    Some(roles.outline_variant),
                    {
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_keychain_editor(cx);
                            });
                        }
                    },
                ))
                .into_any_element(),
            div()
                .id("keychain-editor-footer-import")
                .child(icon_button(
                    AppIcon::Upload,
                    36.0,
                    12.0,
                    Some(if store_available {
                        roles.primary
                    } else {
                        roles.surface_container_highest
                    }),
                    Some(if store_available {
                        roles.on_primary
                    } else {
                        roles.on_surface_variant
                    }),
                    Some(if store_available {
                        roles.primary
                    } else {
                        roles.outline_variant
                    }),
                    {
                        let entity = entity.clone();
                        move |window, cx| {
                            if !store_available {
                                return;
                            }

                            entity.update(cx, |this, cx| {
                                this.import_managed_key(window, cx);
                            });
                        }
                    },
                ))
                .into_any_element(),
        ]);

        let deploy_footer = editor_footer_actions(vec![
            if deploy_in_progress {
                div()
                    .id("keychain-editor-footer-deploy-spinner")
                    .min_w(px(116.0))
                    .min_h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(md3_spinner(18.0))
                    .into_any_element()
            } else {
                editor_button_with_id(
                    "keychain-editor-footer-deploy",
                    i18n::string("keychain.deploy.deploy_button"),
                    true,
                    true,
                    selected_deploy_key.is_none() || deployable_profile_count == 0,
                    {
                        let entity = entity.clone();
                        move |window, cx| {
                            entity.update(cx, |this, cx| {
                                this.deploy_managed_key(window, cx);
                            });
                        }
                    },
                )
                .into_any_element()
            },
            div()
                .id("keychain-deploy-footer-cancel")
                .child(icon_button(
                    AppIcon::Close,
                    36.0,
                    12.0,
                    None,
                    None,
                    Some(roles.outline_variant),
                    {
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_keychain_editor(cx);
                            });
                        }
                    },
                ))
                .into_any_element(),
        ]);

        div()
            .id("keychain-editor-sidebar")
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
