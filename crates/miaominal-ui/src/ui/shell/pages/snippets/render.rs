use crate::ui::components::{SectionCard, editor_button};
use crate::ui::i18n;

use super::super::super::*;
use super::super::empty_state::{shell_empty_page, shell_empty_state};

const SNIPPET_CARD_WIDTH: f32 = 332.0;
const SNIPPET_EDITOR_HEIGHT: f32 = 240.0;

fn snippets_truncate(value: &str, max_chars: usize) -> String {
    let chars: Vec<_> = value.chars().collect();
    if chars.len() <= max_chars {
        return value.to_string();
    }

    let visible: String = chars
        .into_iter()
        .take(max_chars.saturating_sub(3))
        .collect();
    format!("{visible}...")
}

fn snippet_script_preview(script: &str) -> String {
    script
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| snippets_truncate(line, 44))
        .unwrap_or_else(|| i18n::string("snippets.preview.empty"))
}

fn snippets_card_shell(width: f32) -> Div {
    let roles = miaominal_settings::current_theme().material.roles;

    card_surface(roles.surface_container, 20.0)
        .w(px(width))
        .min_h(px(88.0))
        .p_4()
}

fn snippet_package_card(
    title: String,
    snippet_count: usize,
    is_selected: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let palette = group_accent_palette(&title, &material);
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let count = snippet_count.to_string();
    let count_label = if snippet_count == 1 {
        i18n::string_args("snippets.package_card.count_one", &[("count", &count)])
    } else {
        i18n::string_args("snippets.package_card.count_other", &[("count", &count)])
    };
    let icon = miaominal_core::snippet::package_initials(&title)
        .unwrap_or_else(|| i18n::string("snippets.package_card.fallback_icon"));

    card_surface(
        if is_selected {
            palette.accent_container
        } else {
            roles.surface_container
        },
        20.0,
    )
    .w(px(SNIPPET_CARD_WIDTH))
    .min_h(px(88.0))
    .cursor_pointer()
    .p_4()
    .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, cx| {
        on_click(window, cx);
    })
    .child(
        h_flex()
            .size_full()
            .items_center()
            .gap_3()
            .child(
                div()
                    .size(px(44.0))
                    .rounded(px(14.0))
                    .bg(if is_selected {
                        rgb(palette.accent)
                    } else {
                        color_with_alpha(palette.accent, 0x28)
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(miaominal_settings::scaled_font_size(12.0))
                    .text_color(rgb(if is_selected {
                        palette.on_accent
                    } else {
                        palette.accent
                    }))
                    .child(icon),
            )
            .child(
                v_flex()
                    .min_w(px(0.0))
                    .gap_1()
                    .child(
                        div()
                            .text_size(miaominal_settings::scaled_font_size(16.0))
                            .text_color(rgb(if is_selected {
                                palette.on_accent_container
                            } else {
                                roles.on_surface
                            }))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(miaominal_settings::scaled_font_size(12.0))
                            .text_color(rgb(text_muted))
                            .child(count_label),
                    ),
            ),
    )
}

fn snippet_command_card(
    snippet: &SnippetRecord,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let preview = snippet_script_preview(&snippet.script);

    snippets_card_shell(SNIPPET_CARD_WIDTH)
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, cx| {
            on_click(window, cx);
        })
        .child(
            h_flex()
                .size_full()
                .items_center()
                .gap_3()
                .child(page_primary_icon_tile(AppIcon::Notebook, 44.0, 14.0))
                .child(
                    v_flex()
                        .min_w(px(0.0))
                        .gap_2()
                        .child(
                            div()
                                .text_size(miaominal_settings::scaled_font_size(15.0))
                                .line_height(miaominal_settings::scaled_line_height(20.0))
                                .text_color(rgb(roles.on_surface))
                                .child(snippet.description.clone()),
                        )
                        .child(
                            div()
                                .text_size(miaominal_settings::scaled_font_size(11.0))
                                .line_height(miaominal_settings::scaled_line_height(16.0))
                                .text_color(rgb(text_muted))
                                .child(preview),
                        ),
                ),
        )
}

fn snippet_list_row(
    snippet: &SnippetRecord,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let preview = snippet_script_preview(&snippet.script);
    let package_label =
        (!snippet.package.trim().is_empty()).then(|| snippets_truncate(snippet.package.trim(), 20));
    let language_label =
        (!snippet.language.trim().is_empty()).then(|| snippet.language.trim().to_string());

    list_item_card(
        page_primary_icon_tile(AppIcon::Notebook, 44.0, 14.0).into_any_element(),
        v_flex()
            .flex_1()
            .min_w(px(0.0))
            .gap_1()
            .child(
                div()
                    .text_size(miaominal_settings::scaled_font_size(13.0))
                    .text_color(rgb(roles.on_surface))
                    .child(snippet.description.clone()),
            )
            .child(
                div()
                    .text_size(miaominal_settings::scaled_font_size(11.0))
                    .text_color(rgb(text_muted))
                    .child(preview),
            )
            .into_any_element(),
        Some(
            v_flex()
                .items_end()
                .gap_1()
                .flex_shrink_0()
                .when_some(package_label, |this, package| {
                    this.child(
                        div()
                            .text_size(miaominal_settings::scaled_font_size(11.0))
                            .text_color(rgb(roles.on_surface_variant))
                            .child(package),
                    )
                })
                .when_some(language_label, |this, language| {
                    this.child(
                        div()
                            .text_size(miaominal_settings::scaled_font_size(11.0))
                            .text_color(rgb(text_muted))
                            .child(language),
                    )
                })
                .into_any_element(),
        ),
        None,
        move |window, cx| {
            on_click(window, cx);
        },
    )
}

fn snippets_empty_state(message: impl Into<SharedString>) -> impl IntoElement {
    shell_empty_state(AppIcon::Notebook, message)
}

impl AppView {
    pub(in crate::ui::shell) fn render_snippets_page(
        &self,
        entity: Entity<Self>,
        cx: &App,
    ) -> gpui::AnyElement {
        let filter_text = self
            .panel_forms
            .snippets
            .filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let search_matched_snippets: Vec<_> = self
            .data
            .snippets
            .iter()
            .enumerate()
            .filter(|(_, snippet)| miaominal_core::snippet::matches_filter(snippet, &filter_text))
            .collect();
        let selected_package_filter = self.panel_view.snippets_package_filter.as_deref();
        let mut package_summaries: Vec<_> = search_matched_snippets
            .iter()
            .filter_map(|(_, snippet)| {
                let package = snippet.package.trim();
                (!package.is_empty()).then(|| package.to_string())
            })
            .collect();
        package_summaries.sort_by_key(|package| package.to_ascii_lowercase());
        package_summaries.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        let package_summaries: Vec<_> = package_summaries
            .into_iter()
            .map(|package| {
                let count = search_matched_snippets
                    .iter()
                    .filter(|(_, snippet)| snippet.package.eq_ignore_ascii_case(package.as_str()))
                    .count();
                (package, count)
            })
            .collect();

        let mut visible_snippets: Vec<_> = search_matched_snippets
            .iter()
            .copied()
            .filter(|(_, snippet)| {
                selected_package_filter
                    .is_none_or(|package| snippet.package.eq_ignore_ascii_case(package))
            })
            .collect();
        visible_snippets.sort_by(|(_, left), (_, right)| {
            left.description
                .to_ascii_lowercase()
                .cmp(&right.description.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });

        if self.data.snippets.is_empty() {
            return shell_empty_page(
                AppIcon::Notebook,
                i18n::string("snippets.empty.no_snippets"),
            )
            .into_any_element();
        }

        let is_list = self.panel_view.snippets_view_mode == ProfileViewMode::List;

        let header = v_flex()
            .w_full()
            .min_w(px(0.0))
            .gap_6()
            .px_5()
            .child(
                h_flex().w_full().min_w(px(0.0)).justify_center().child(
                    h_flex()
                        .w_full()
                        .min_w(px(0.0))
                        .max_w(px(576.0))
                        .child(search_filter_input(
                            &self.panel_forms.snippets.filter_input,
                            SearchInputStyle::Pill,
                            None,
                        )),
                ),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w(px(0.0))
                    .justify_end()
                    .gap_2()
                    .child(page_view_mode_toolbar_item(AppIcon::Grid, !is_list, {
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.handle_snippets_view_mode_change(ProfileViewMode::Grid, cx);
                            });
                        }
                    }))
                    .child(page_view_mode_toolbar_item(AppIcon::List, is_list, {
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.handle_snippets_view_mode_change(ProfileViewMode::List, cx);
                            });
                        }
                    })),
            );

        let content = v_flex()
            .w_full()
            .min_w(px(0.0))
            .gap_6()
            .px_5()
            .pb_8()
            .when(!package_summaries.is_empty(), |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .min_w(px(0.0))
                        .gap_2()
                        .child(page_section_title(i18n::string("snippets.page.packages")))
                        .child(
                            div()
                                .w_full()
                                .min_w(px(0.0))
                                .flex()
                                .flex_wrap()
                                .gap_4()
                                .children(package_summaries.into_iter().map(|(package, count)| {
                                    let package_name = package.clone();
                                    let is_selected =
                                        selected_package_filter.is_some_and(|selected| {
                                            selected.eq_ignore_ascii_case(package_name.as_str())
                                        });
                                    snippet_package_card(package, count, is_selected, {
                                        let entity = entity.clone();
                                        move |_, cx| {
                                            let package_name = package_name.clone();
                                            entity.update(cx, |this, cx| {
                                                this.handle_snippets_package_filter_toggle(
                                                    package_name.clone(),
                                                    cx,
                                                );
                                            });
                                        }
                                    })
                                })),
                        ),
                )
            })
            .child(
                v_flex()
                    .w_full()
                    .min_w(px(0.0))
                    .gap_4()
                    .child(
                        h_flex()
                            .w_full()
                            .min_w(px(0.0))
                            .items_center()
                            .gap_3()
                            .child(page_section_title(i18n::string("snippets.page.snippets"))),
                    )
                    .child(if search_matched_snippets.is_empty() {
                        snippets_empty_state(i18n::string("snippets.empty.no_search_matches"))
                            .into_any_element()
                    } else if visible_snippets.is_empty() {
                        snippets_empty_state(i18n::string("snippets.empty.no_package_matches"))
                            .into_any_element()
                    } else if self.panel_view.snippets_view_mode == ProfileViewMode::List {
                        v_flex()
                            .w_full()
                            .min_w(px(0.0))
                            .gap_2()
                            .children(visible_snippets.into_iter().map(|(index, snippet)| {
                                let entity = entity.clone();
                                snippet_list_row(snippet, move |window, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.open_existing_snippet_editor(index, window, cx);
                                    });
                                })
                                .into_any_element()
                            }))
                            .into_any_element()
                    } else {
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .flex()
                            .flex_wrap()
                            .gap_4()
                            .children(visible_snippets.into_iter().map(|(index, snippet)| {
                                let entity = entity.clone();
                                snippet_command_card(snippet, move |window, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.open_existing_snippet_editor(index, window, cx);
                                    });
                                })
                                .into_any_element()
                            }))
                            .into_any_element()
                    }),
            );

        div()
            .size_full()
            .overflow_hidden()
            .child(
                v_flex()
                    .size_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .gap_6()
                    .child(header)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .child(div().size_full().overflow_y_scrollbar().child(content)),
                    ),
            )
            .into_any_element()
    }

    pub(in crate::ui::shell) fn render_snippets_fab(
        &self,
        entity: Entity<Self>,
    ) -> impl IntoElement {
        fab_button(move |window, cx| {
            entity.update(cx, |this, cx| this.open_snippets_editor(window, cx));
        })
    }

    pub(in crate::ui::shell) fn render_snippets_editor_sidebar(
        &self,
        entity: Entity<Self>,
    ) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let is_editing = self
            .data
            .selected_snippet
            .and_then(|index| self.data.snippets.get(index))
            .is_some();
        let available_packages = Self::collect_available_snippet_packages(&self.data.snippets);
        let forms = &self.panel_forms.snippets;
        let editor_title = if is_editing {
            i18n::string("snippets.editor.edit_title")
        } else {
            i18n::string("snippets.editor.create_title")
        };
        let header = h_flex().w_full().items_start().gap_4().child(
            v_flex().flex_1().gap_1().child(
                div()
                    .text_size(miaominal_settings::scaled_font_size(20.0))
                    .text_color(rgb(roles.on_surface))
                    .child(editor_title),
            ),
        );

        let save_entity = entity.clone();
        let mut footer_actions = Vec::new();
        if is_editing {
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
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.delete_selected_snippet(cx);
                            });
                        }
                    },
                )
                .into_any_element(),
            );
        }
        footer_actions.push(
            editor_button(i18n::string("snippets.editor.cancel"), false, true, {
                let entity = entity.clone();
                move |_, cx| {
                    entity.update(cx, |this, cx| {
                        this.close_snippets_editor(cx);
                    });
                }
            })
            .into_any_element(),
        );
        footer_actions.push(
            editor_button(
                i18n::string("snippets.editor.save"),
                true,
                true,
                move |window, cx| {
                    save_entity.update(cx, |this, cx| {
                        this.save_snippet(window, cx);
                    });
                },
            )
            .into_any_element(),
        );
        let footer = editor_footer_actions(footer_actions);

        div()
            .id("snippets-editor-sidebar")
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
                                    .child(SectionCard::new(
                                        AppIcon::Notebook,
                                        i18n::string("snippets.editor.general"),
                                        v_flex()
                                            .w_full()
                                            .gap_4()
                                            .child(
                                                div()
                                                    .text_size(miaominal_settings::scaled_font_size(12.0))
                                                    .line_height(miaominal_settings::scaled_line_height(18.0))
                                                    .text_color(rgb(roles.on_surface_variant))
                                                    .child(i18n::string(
                                                        "snippets.editor.description",
                                                    )),
                                            )
                                            .child(surface_text_input_stack(
                                                i18n::string(
                                                    "snippets.editor.action_description",
                                                ),
                                                forms.description_input.clone(),
                                                TextInputSurface::Low,
                                                true,
                                            ))
                                            .child(
                                                v_flex()
                                                    .w_full()
                                                    .gap_2()
                                                    .child(field_label(
                                                        i18n::string("snippets.editor.package"),
                                                        false,
                                                    ))
                                                    .child(
                                                        h_flex()
                                                            .w_full()
                                                            .items_center()
                                                            .gap_2()
                                                            .child(
                                                                Select::new(&forms.package_select)
                                                                    .large()
                                                                    .w_full()
                                                                    .rounded(px(14.0))
                                                                    .border_0()
                                                                    .bg(rgb(roles.surface_container_low))
                                                                    .cleanable(true)
                                                                    .placeholder(
                                                                        if available_packages.is_empty() {
                                                                            i18n::string(
                                                                                "snippets.editor.no_existing_packages",
                                                                            )
                                                                        } else {
                                                                            i18n::string(
                                                                                "snippets.editor.select_existing_package",
                                                                            )
                                                                        },
                                                                    )
                                                                    .disabled(
                                                                        available_packages.is_empty(),
                                                                    ),
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
                                                                            this.begin_new_snippet_package(
                                                                                window,
                                                                                cx,
                                                                            );
                                                                        });
                                                                    }
                                                                },
                                                            )),
                                                    )
                                                    .when(forms.creating_new_package, |this| {
                                                        this.child(
                                                            surface_text_input(
                                                                &forms.package_input,
                                                                TextInputSurface::Low,
                                                            )
                                                            .large(),
                                                        )
                                                    }),
                                            )
                                            .child(surface_text_editor_stack(
                                                i18n::string("snippets.editor.script"),
                                                forms.script_input.clone(),
                                                SNIPPET_EDITOR_HEIGHT,
                                                TextInputSurface::Low,
                                                true,
                                            )),
                                    )),
                            ),
                        ),
                    )
                    .child(div().px_4().py_4().child(footer)),
            )
    }
}
