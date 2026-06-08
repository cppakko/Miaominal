use super::super::super::*;
use super::super::empty_state::shell_empty_page;
use crate::ui::i18n;
use miaominal_core::known_host::KnownHostEntry;

#[derive(Clone, Debug, PartialEq, Eq)]
struct LinkedTrustedProfile {
    id: String,
    name: String,
    summary: String,
}

#[derive(Clone, Debug)]
struct TrustedKnownHostView {
    entry: KnownHostEntry,
    linked_profiles: Vec<LinkedTrustedProfile>,
    duplicate_count: usize,
}

impl TrustedKnownHostView {
    fn key_matches(&self, host: &str, port: u16, fingerprint: &str) -> bool {
        self.entry.host == host && self.entry.port == port && self.entry.fingerprint == fingerprint
    }

    fn is_orphaned(&self) -> bool {
        self.linked_profiles.is_empty()
    }

    fn address(&self) -> String {
        format!("{}:{}", self.entry.host, self.entry.port)
    }

    fn primary_profile_name(&self) -> String {
        self.linked_profiles
            .first()
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| i18n::string("trusted.page.unlinked"))
    }
}

fn derive_trusted_known_hosts(
    entries: &[KnownHostEntry],
    sessions: &[SessionProfile],
) -> Vec<TrustedKnownHostView> {
    entries
        .iter()
        .map(|entry| {
            let linked_profiles = sessions
                .iter()
                .filter(|profile| profile.host == entry.host && profile.port == entry.port)
                .map(|profile| LinkedTrustedProfile {
                    id: profile.id.clone(),
                    name: profile.connection_label(),
                    summary: profile.summary(),
                })
                .collect();
            let duplicate_count = entries
                .iter()
                .filter(|candidate| candidate.host == entry.host && candidate.port == entry.port)
                .count();

            TrustedKnownHostView {
                entry: entry.clone(),
                linked_profiles,
                duplicate_count,
            }
        })
        .collect()
}

fn trusted_host_matches_query(view: &TrustedKnownHostView, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }

    let mut haystack = format!(
        "{} {} {} {} {}",
        view.entry.host,
        view.entry.port,
        view.entry.algorithm,
        view.entry.fingerprint,
        view.address()
    )
    .to_lowercase();
    for profile in &view.linked_profiles {
        haystack.push(' ');
        haystack.push_str(&profile.name.to_lowercase());
        haystack.push(' ');
        haystack.push_str(&profile.summary.to_lowercase());
    }

    haystack.contains(&query)
}

fn trusted_host_matches_filter(view: &TrustedKnownHostView, filter: TrustedHostFilter) -> bool {
    match filter {
        TrustedHostFilter::All => true,
        TrustedHostFilter::Linked => !view.is_orphaned(),
        TrustedHostFilter::Orphaned => view.is_orphaned(),
        TrustedHostFilter::DefaultPort => view.entry.port == 22,
        TrustedHostFilter::CustomPort => view.entry.port != 22,
    }
}

fn selected_trusted_host<'a>(
    views: &'a [TrustedKnownHostView],
    selected: Option<&(String, u16, String)>,
) -> Option<&'a TrustedKnownHostView> {
    selected.and_then(|(host, port, fingerprint)| {
        views
            .iter()
            .find(|view| view.key_matches(host, *port, fingerprint))
    })
}

fn trusted_filter_label(filter: TrustedHostFilter) -> String {
    i18n::string(match filter {
        TrustedHostFilter::All => "trusted.filters.all",
        TrustedHostFilter::Linked => "trusted.filters.linked",
        TrustedHostFilter::Orphaned => "trusted.filters.orphaned",
        TrustedHostFilter::DefaultPort => "trusted.filters.default_port",
        TrustedHostFilter::CustomPort => "trusted.filters.custom_port",
    })
}

fn trusted_filter_button(
    entity: Entity<AppView>,
    filter: TrustedHostFilter,
    active: bool,
) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let background = if active {
        roles.secondary_container
    } else {
        roles.surface_container_high
    };
    let foreground = if active {
        roles.on_secondary_container
    } else {
        roles.on_surface_variant
    };

    div()
        .px_3()
        .h(px(32.0))
        .rounded(px(16.0))
        .bg(rgb(background))
        .flex()
        .items_center()
        .cursor_pointer()
        .text_size(miaominal_settings::FontSize::Body.scaled())
        .text_color(rgb(foreground))
        .child(trusted_filter_label(filter))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.handle_trusted_host_filter_change(filter, cx);
            });
        })
}

fn trusted_stat_tile(label: String, value: String, tone: u32) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;

    div()
        .min_w(px(112.0))
        .rounded(px(8.0))
        .bg(rgb(roles.surface_container_high))
        .p_3()
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(roles.on_surface_variant))
                        .child(label),
                )
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Subtitle.scaled())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(tone))
                        .child(value),
                ),
        )
}

fn risk_badge(view: &TrustedKnownHostView) -> Option<gpui::AnyElement> {
    let roles = miaominal_settings::current_theme().material.roles;

    if view.duplicate_count > 1 {
        Some(
            badge(
                i18n::string_args(
                    "trusted.badges.duplicates",
                    &[("count", &view.duplicate_count.to_string())],
                ),
                roles.surface_container_high,
                roles.error,
            )
            .into_any_element(),
        )
    } else {
        None
    }
}

fn linked_profile_badges(view: &TrustedKnownHostView) -> Vec<gpui::AnyElement> {
    let roles = miaominal_settings::current_theme().material.roles;
    if view.linked_profiles.is_empty() {
        return Vec::new();
    }

    let mut badges: Vec<gpui::AnyElement> = view
        .linked_profiles
        .iter()
        .take(2)
        .map(|profile| {
            badge(
                profile.name.clone(),
                roles.secondary_container,
                roles.on_secondary_container,
            )
            .into_any_element()
        })
        .collect();
    if view.linked_profiles.len() > 2 {
        badges.push(
            badge(
                i18n::string_args(
                    "trusted.page.more_profiles",
                    &[("count", &(view.linked_profiles.len() - 2).to_string())],
                ),
                roles.surface_container_high,
                roles.on_surface_variant,
            )
            .into_any_element(),
        );
    }
    badges
}

fn trusted_host_card(entity: Entity<AppView>, view: TrustedKnownHostView) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let address = view.address();
    let select_host = view.entry.host.clone();
    let select_fingerprint = view.entry.fingerprint.clone();
    let copy_host = view.entry.host.clone();
    let port = view.entry.port;

    let mut badges_row = h_flex()
        .gap_2()
        .flex_wrap()
        .child(badge(
            view.entry.algorithm.clone(),
            roles.surface_container_high,
            roles.on_surface_variant,
        ))
        .child(badge(
            i18n::string_args("trusted.page.port_badge", &[("port", &port.to_string())]),
            roles.surface_container_high,
            roles.on_surface_variant,
        ));
    if let Some(risk) = risk_badge(&view) {
        badges_row = badges_row.child(risk);
    }

    card_surface(roles.surface_container, 20.0)
        .w(px(TRUSTED_CARD_WIDTH))
        .min_h(px(128.0))
        .p_4()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, {
            let entity = entity.clone();
            move |_, _, cx| {
                let host = select_host.clone();
                let fingerprint = select_fingerprint.clone();
                entity.update(cx, |this, cx| {
                    this.select_trusted_known_host(host, port, fingerprint, cx);
                });
            }
        })
        .child(
            h_flex()
                .size_full()
                .gap_3()
                .items_start()
                .child(page_primary_icon_tile(AppIcon::FingerPrint, 44.0, 14.0))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_3()
                        .child(
                            div()
                                .min_w(px(0.0))
                                .text_size(miaominal_settings::FontSize::Subtitle.scaled())
                                .line_height(miaominal_settings::scaled_line_height(20.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(roles.on_surface))
                                .child(address),
                        )
                        .child(badges_row)
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .children(linked_profile_badges(&view)),
                        ),
                ),
        )
        .on_mouse_up(MouseButton::Right, move |_, _, cx| {
            let host = copy_host.clone();
            entity.update(cx, |this, cx| {
                this.copy_known_host_address(host, port, cx);
            });
        })
}

fn trusted_detail_panel(
    entity: Entity<AppView>,
    view: &TrustedKnownHostView,
    path: String,
) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let address = view.address();
    let copy_fingerprint = view.entry.fingerprint.clone();
    let copy_address_host = view.entry.host.clone();
    let remove_host = view.entry.host.clone();
    let port = view.entry.port;
    let close_footer_entity = entity.clone();

    let mut linked_profiles = v_flex().gap_2();
    if view.linked_profiles.is_empty() {
        linked_profiles = linked_profiles.child(
            div()
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .text_color(rgb(roles.on_surface_variant))
                .child(i18n::string("trusted.details.no_profiles")),
        );
    } else {
        for profile in &view.linked_profiles {
            let open_id = profile.id.clone();
            linked_profiles = linked_profiles.child(
                div()
                    .rounded(px(8.0))
                    .bg(rgb(roles.surface_container_high))
                    .p_3()
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(miaominal_settings::FontSize::Input.scaled())
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(roles.on_surface))
                                            .child(profile.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(miaominal_settings::FontSize::Body.scaled())
                                            .text_color(rgb(roles.on_surface_variant))
                                            .child(profile.summary.clone()),
                                    ),
                            )
                            .child(icon_button(
                                AppIcon::Edit,
                                30.0,
                                8.0,
                                Some(roles.surface_container),
                                Some(roles.on_surface_variant),
                                None,
                                {
                                    let entity = entity.clone();
                                    move |window, cx| {
                                        let profile_id = open_id.clone();
                                        entity.update(cx, |this, cx| {
                                            this.open_linked_profile_from_known_host(
                                                profile_id, window, cx,
                                            );
                                        });
                                    }
                                },
                            )),
                    ),
            );
        }
    }

    let header = h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .child(page_primary_icon_tile(AppIcon::FingerPrint, 48.0, 14.0))
        .child(
            v_flex()
                .flex_1()
                .min_w(px(0.0))
                .gap_1()
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Subheading.scaled())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(roles.on_surface))
                        .child(address.clone()),
                )
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(roles.on_surface_variant))
                        .child(view.primary_profile_name()),
                ),
        );

    let footer = editor_footer_actions(vec![
        editor_button_with_id(
            "known-host-copy-fingerprint",
            i18n::string("trusted.actions.copy_fingerprint"),
            false,
            true,
            false,
            {
                let entity = entity.clone();
                move |_, cx| {
                    let fingerprint = copy_fingerprint.clone();
                    entity.update(cx, |this, cx| {
                        this.copy_known_host_fingerprint(fingerprint, cx);
                    });
                }
            },
        )
        .into_any_element(),
        editor_button_with_id(
            "known-host-copy-address",
            i18n::string("trusted.actions.copy_address"),
            false,
            true,
            false,
            {
                let entity = entity.clone();
                move |_, cx| {
                    let host = copy_address_host.clone();
                    entity.update(cx, |this, cx| {
                        this.copy_known_host_address(host, port, cx);
                    });
                }
            },
        )
        .into_any_element(),
        div()
            .id("known-host-delete-detail")
            .child(icon_button(
                AppIcon::Trash,
                EDITOR_FOOTER_ACTION_HEIGHT,
                12.0,
                Some(roles.error),
                Some(roles.on_error),
                Some(roles.error),
                move |_, cx| {
                    let host = remove_host.clone();
                    entity.update(cx, |this, cx| {
                        this.request_trusted_known_host_removal(host, port, cx);
                    });
                },
            ))
            .into_any_element(),
        div()
            .id("known-host-sidebar-close")
            .child(icon_button(
                AppIcon::Close,
                EDITOR_FOOTER_ACTION_HEIGHT,
                12.0,
                None,
                None,
                Some(roles.outline_variant),
                move |_, cx| {
                    close_footer_entity.update(cx, |this, cx| {
                        this.close_trusted_known_host_sidebar(cx);
                    });
                },
            ))
            .into_any_element(),
    ]);

    div()
        .id("known-hosts-detail-sidebar")
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
                                .gap_5()
                                .pb_4()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .flex_wrap()
                                        .child(badge(
                                            view.entry.algorithm.clone(),
                                            roles.surface_container_high,
                                            roles.on_surface_variant,
                                        ))
                                        .child(badge(
                                            i18n::string_args(
                                                "trusted.page.port_badge",
                                                &[("port", &view.entry.port.to_string())],
                                            ),
                                            roles.surface_container_high,
                                            roles.on_surface_variant,
                                        ))
                                        .when_some(risk_badge(view), |this, risk| this.child(risk)),
                                )
                                .child(
                                    div()
                                        .rounded(px(8.0))
                                        .bg(rgb(roles.surface_container_high))
                                        .p_4()
                                        .child(
                                            v_flex()
                                                .gap_3()
                                                .child(detail_row(
                                                    i18n::string("trusted.details.algorithm"),
                                                    &view.entry.algorithm,
                                                ))
                                                .child(detail_row(
                                                    i18n::string(
                                                        "trusted.details.fingerprint_sha256",
                                                    ),
                                                    &view.entry.fingerprint,
                                                ))
                                                .child(detail_row(
                                                    i18n::string("trusted.details.file_path"),
                                                    &path,
                                                )),
                                        ),
                                )
                                .child(
                                    v_flex()
                                        .gap_3()
                                        .child(page_section_title(i18n::string(
                                            "trusted.details.linked_profiles",
                                        )))
                                        .child(linked_profiles),
                                ),
                        ),
                    ),
                )
                .child(div().px_4().pt_3().pb_4().child(footer)),
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
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let entries = self.data.known_hosts_entries.clone();
        let views = derive_trusted_known_hosts(&entries, &self.data.sessions);
        let total = views.len();
        let linked = views.iter().filter(|view| !view.is_orphaned()).count();
        let orphaned = views.iter().filter(|view| view.is_orphaned()).count();
        let duplicated = views.iter().filter(|view| view.duplicate_count > 1).count();
        if views.is_empty() {
            return shell_empty_page(AppIcon::FingerPrint, i18n::string("trusted.page.empty"))
                .child(
                    div()
                        .max_w(px(460.0))
                        .pt_3()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .line_height(miaominal_settings::scaled_line_height(20.0))
                        .text_color(rgb(miaominal_settings::current_theme()
                            .material
                            .roles
                            .on_surface_variant))
                        .child(i18n::string("trusted.page.empty_detail")),
                )
                .into_any_element();
        }

        let query = self
            .panel_forms
            .trusted
            .filter_input
            .read(cx)
            .value()
            .to_string();
        let filter = self.panel_view.trusted_host_filter;
        let filtered: Vec<_> = views
            .iter()
            .filter(|view| trusted_host_matches_query(view, &query))
            .filter(|view| trusted_host_matches_filter(view, filter))
            .cloned()
            .collect();
        let roles = miaominal_settings::current_theme().material.roles;
        let warning = miaominal_settings::current_theme()
            .material
            .extended
            .warning
            .color;

        div()
            .size_full()
            .p_5()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .size_full()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        v_flex()
                            .gap_4()
                            .child(
                                h_flex().w_full().justify_center().child(
                                    h_flex()
                                        .w_full()
                                        .max_w(px(576.0))
                                        .child(search_filter_input(
                                            &self.panel_forms.trusted.filter_input,
                                            SearchInputStyle::Pill,
                                            None,
                                        )),
                                ),
                            )
                            .child(page_section_title(i18n::string(
                                "trusted.page.trust_center",
                            )))
                            .child(
                                h_flex()
                                    .gap_3()
                                    .flex_wrap()
                                    .child(trusted_stat_tile(
                                        i18n::string("trusted.stats.total"),
                                        total.to_string(),
                                        roles.primary,
                                    ))
                                    .child(trusted_stat_tile(
                                        i18n::string("trusted.stats.linked"),
                                        linked.to_string(),
                                        roles.primary,
                                    ))
                                    .child(trusted_stat_tile(
                                        i18n::string("trusted.stats.orphaned"),
                                        orphaned.to_string(),
                                        warning,
                                    ))
                                    .child(trusted_stat_tile(
                                        i18n::string("trusted.stats.duplicates"),
                                        duplicated.to_string(),
                                        roles.error,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(trusted_filter_button(
                                        entity.clone(),
                                        TrustedHostFilter::All,
                                        filter == TrustedHostFilter::All,
                                    ))
                                    .child(trusted_filter_button(
                                        entity.clone(),
                                        TrustedHostFilter::Linked,
                                        filter == TrustedHostFilter::Linked,
                                    ))
                                    .child(trusted_filter_button(
                                        entity.clone(),
                                        TrustedHostFilter::Orphaned,
                                        filter == TrustedHostFilter::Orphaned,
                                    ))
                                    .child(trusted_filter_button(
                                        entity.clone(),
                                        TrustedHostFilter::DefaultPort,
                                        filter == TrustedHostFilter::DefaultPort,
                                    ))
                                    .child(trusted_filter_button(
                                        entity.clone(),
                                        TrustedHostFilter::CustomPort,
                                        filter == TrustedHostFilter::CustomPort,
                                    )),
                            ),
                    )
                    .child(div().flex_1().min_h_0().overflow_y_scrollbar().child(
                        if filtered.is_empty() {
                            shell_empty_page(
                                AppIcon::FingerPrint,
                                i18n::string("trusted.page.no_matches"),
                            )
                            .into_any_element()
                        } else {
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_4()
                                .children(filtered.into_iter().map(|view| {
                                    trusted_host_card(entity.clone(), view).into_any_element()
                                }))
                                .into_any_element()
                        },
                    )),
            )
            .into_any_element()
    }

    pub(in crate::ui::shell) fn render_known_hosts_refresh_fab(
        &self,
        entity: Entity<AppView>,
    ) -> impl IntoElement {
        fab_icon_button(AppIcon::Rotate, move |_, cx| {
            entity.update(cx, |this, cx| {
                this.refresh_known_hosts();
                this.status_message = i18n::string("trusted.messages.refreshed");
                cx.notify();
            });
        })
    }

    pub(in crate::ui::shell) fn render_trusted_known_host_sidebar(
        &self,
        entity: Entity<AppView>,
    ) -> impl IntoElement {
        let views = derive_trusted_known_hosts(&self.data.known_hosts_entries, &self.data.sessions);
        let path = self.services.known_hosts.path().display().to_string();

        selected_trusted_host(&views, self.panels.selected_known_host.as_ref())
            .map(|view| trusted_detail_panel(entity.clone(), view, path).into_any_element())
            .unwrap_or_else(|| {
                shell_empty_page(
                    AppIcon::FingerPrint,
                    i18n::string("trusted.page.no_matches"),
                )
                .into_any_element()
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn known_host(host: &str, port: u16, algorithm: &str, fingerprint: &str) -> KnownHostEntry {
        KnownHostEntry {
            host: host.into(),
            port,
            algorithm: algorithm.into(),
            fingerprint: fingerprint.into(),
        }
    }

    fn profile(id: &str, name: &str, host: &str, port: u16) -> SessionProfile {
        let mut profile = SessionProfile::blank(id, 1);
        profile.name = name.into();
        profile.host = host.into();
        profile.port = port;
        profile.username = "root".into();
        profile
    }

    #[test]
    fn derives_linked_profiles_by_host_and_port() {
        let entries = vec![known_host("example.com", 22, "ssh-ed25519", "SHA256:a")];
        let sessions = vec![
            profile("one", "Production", "example.com", 22),
            profile("two", "Other", "example.com", 2222),
        ];

        let views = derive_trusted_known_hosts(&entries, &sessions);

        assert_eq!(views[0].linked_profiles.len(), 1);
        assert_eq!(views[0].linked_profiles[0].name, "Production");
        assert!(!views[0].is_orphaned());
    }

    #[test]
    fn marks_orphaned_and_duplicate_entries() {
        let entries = vec![
            known_host("example.com", 22, "ssh-ed25519", "SHA256:a"),
            known_host("example.com", 22, "rsa-sha2-512", "SHA256:b"),
        ];

        let views = derive_trusted_known_hosts(&entries, &[]);

        assert!(views[0].is_orphaned());
        assert_eq!(views[0].duplicate_count, 2);
        assert_eq!(views[1].duplicate_count, 2);
    }

    #[test]
    fn query_matches_host_fingerprint_algorithm_and_profile() {
        let entries = vec![known_host("example.com", 22, "ssh-ed25519", "SHA256:abc")];
        let sessions = vec![profile("one", "Production", "example.com", 22)];
        let views = derive_trusted_known_hosts(&entries, &sessions);

        assert!(trusted_host_matches_query(&views[0], "prod"));
        assert!(trusted_host_matches_query(&views[0], "ed25519"));
        assert!(trusted_host_matches_query(&views[0], "sha256:abc"));
        assert!(!trusted_host_matches_query(&views[0], "staging"));
    }

    #[test]
    fn selected_host_requires_explicit_selection() {
        let entries = vec![known_host("example.com", 22, "ssh-ed25519", "SHA256:abc")];
        let views = derive_trusted_known_hosts(&entries, &[]);

        assert!(selected_trusted_host(&views, None).is_none());
        assert!(
            selected_trusted_host(
                &views,
                Some(&("example.com".into(), 22, "SHA256:abc".into()))
            )
            .is_some()
        );
    }
}
