use crate::ui::assets::AppIcon;
use crate::ui::components::catalog_card::{
    catalog_card_content, catalog_card_icon, catalog_card_subtitle, catalog_card_surface,
    catalog_card_title,
};
use crate::ui::components::empty_state::{empty_page, empty_state};
use crate::ui::components::group_accent::group_accent_palette;
use crate::ui::components::metrics::EDITOR_DRAWER_WIDTH;
use crate::ui::components::shell_tokens::{ShellMetrics, space};
use crate::ui::components::tokens::color_with_alpha;
use crate::ui::components::{
    EditorFooterActionTone, SectionCard, editor_footer_button, md3_select,
};
use crate::ui::components::{
    TextInputSurface, dock_action_button, editor_footer_actions, field_label, icon_button,
    list_item_card, page_section_title, surface_text_editor_stack, surface_text_input,
    surface_text_input_stack, view_mode_switcher,
};
use crate::ui::components::{animated_card_collection, card_grid, card_grid_columns};
use crate::ui::contracts::navigation::ProfileViewMode;
use crate::ui::features::snippets::SnippetsController;
use crate::ui::i18n;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, MouseButton, SharedString, Window, div, px, rgb};
use miaominal_core::snippet::SnippetRecord;

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

fn snippet_package_card(
    title: String,
    snippet_count: usize,
    is_selected: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let material = miaominal_settings::current_theme().material;
    let palette = group_accent_palette(&title, &material);
    let count = snippet_count.to_string();
    let count_label = if snippet_count == 1 {
        i18n::string_args("snippets.package_card.count_one", &[("count", &count)])
    } else {
        i18n::string_args("snippets.package_card.count_other", &[("count", &count)])
    };
    let item_id = SharedString::from(format!("snippet-package-card-{}", title.as_str()));
    let icon = miaominal_core::snippet::package_initials(&title)
        .unwrap_or_else(|| i18n::string("snippets.package_card.fallback_icon"));

    let mut card = catalog_card_surface()
        .id(item_id)
        .test_support()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, cx| {
            on_click(window, cx);
        });
    if is_selected {
        card = card.bg(rgb(palette.accent_container));
    }
    card.child(catalog_card_content(
        div()
            .size(px(ShellMetrics::CARD_ICON_SIZE))
            .flex_shrink_0()
            .rounded(px(ShellMetrics::CARD_ICON_RADIUS))
            .bg(if is_selected {
                rgb(palette.accent)
            } else {
                color_with_alpha(palette.accent, 0x28)
            })
            .flex()
            .items_center()
            .justify_center()
            .text_size(miaominal_settings::FontSize::Input.scaled())
            .text_color(rgb(if is_selected {
                palette.on_accent
            } else {
                palette.accent
            }))
            .child(icon),
        catalog_card_title(title).when(is_selected, |title| {
            title.text_color(rgb(palette.on_accent_container))
        }),
        Some(count_label.into()),
        None,
        None,
    ))
}

fn snippet_command_card(
    snippet: &SnippetRecord,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let preview = snippet
        .script
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| i18n::string("snippets.preview.empty"));
    let item_id = SharedString::from(format!("snippet-card-{}", snippet.id.as_str()));

    catalog_card_surface()
        .id(item_id)
        .test_support()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, cx| {
            on_click(window, cx);
        })
        .child(catalog_card_content(
            catalog_card_icon(AppIcon::Notebook),
            catalog_card_title(snippet.description.clone()),
            Some(preview.into()),
            None,
            None,
        ))
}

fn snippet_list_row(
    snippet: &SnippetRecord,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let preview = snippet_script_preview(&snippet.script);
    let package_label =
        (!snippet.package.trim().is_empty()).then(|| snippets_truncate(snippet.package.trim(), 20));
    let language_label =
        (!snippet.language.trim().is_empty()).then(|| snippet.language.trim().to_string());
    let item_id = SharedString::from(format!("snippet-row-{}", snippet.id.as_str()));

    list_item_card(
        catalog_card_icon(AppIcon::Notebook).into_any_element(),
        v_flex()
            .flex_1()
            .min_w(px(0.0))
            .justify_center()
            .child(catalog_card_title(snippet.description.clone()))
            .child(catalog_card_subtitle(preview))
            .into_any_element(),
        Some(
            v_flex()
                .items_end()
                .gap_1()
                .flex_shrink_0()
                .when_some(package_label, |this, package| {
                    this.child(
                        div()
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .text_color(rgb(roles.on_surface_variant))
                            .child(package),
                    )
                })
                .when_some(language_label, |this, language| {
                    this.child(catalog_card_subtitle(language))
                })
                .into_any_element(),
        ),
        None,
        move |window, cx| {
            on_click(window, cx);
        },
    )
    .id(item_id)
    .test_support()
}

fn snippets_empty_state(message: impl Into<SharedString>) -> impl IntoElement {
    empty_state(AppIcon::Notebook, message)
}

impl SnippetsController {
    pub(in crate::ui) fn render_snippets_page(
        &self,
        controller: Entity<Self>,
        on_open_existing: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
        available_width: f32,
        _cx: &App,
    ) -> gpui_kit::AnyElement {
        let catalog_view = self.catalog_view();
        let snippets = self.snippets().clone();
        let selected_package_filter = catalog_view.snippets_package_filter.as_deref();
        let mut package_summaries: Vec<_> = snippets
            .iter()
            .filter_map(|snippet| {
                let package = snippet.package.trim();
                (!package.is_empty()).then(|| package.to_string())
            })
            .collect();
        package_summaries.sort_by_key(|package| package.to_ascii_lowercase());
        package_summaries.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        let package_summaries: Vec<_> = package_summaries
            .into_iter()
            .map(|package| {
                let count = snippets
                    .iter()
                    .filter(|snippet| snippet.package.eq_ignore_ascii_case(package.as_str()))
                    .count();
                (package, count)
            })
            .collect();

        let mut visible_snippets: Vec<_> = snippets
            .iter()
            .filter(|snippet| {
                selected_package_filter
                    .is_none_or(|package| snippet.package.eq_ignore_ascii_case(package))
            })
            .collect();
        visible_snippets.sort_by(|left, right| {
            left.description
                .to_ascii_lowercase()
                .cmp(&right.description.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });

        if snippets.is_empty() {
            return empty_page(
                AppIcon::Notebook,
                i18n::string("snippets.empty.no_snippets"),
            )
            .into_any_element();
        }

        let is_list = catalog_view.snippets_view_mode == ProfileViewMode::List;
        let header = v_flex()
            .w_full()
            .min_w(px(0.0))
            .gap_6()
            .px(space(ShellMetrics::CATALOG_INSET))
            .child(
                h_flex()
                    .w_full()
                    .min_w(px(0.0))
                    .max_w(space(ShellMetrics::CONTENT_MAX_WIDTH))
                    .mx_auto()
                    .justify_end()
                    .child(view_mode_switcher(
                        "snippets-view-mode",
                        catalog_view.snippets_view_mode,
                        {
                            let controller = controller.clone();
                            move |mode, _, cx| {
                                controller.update(cx, |controller, cx| {
                                    controller.set_snippets_view_mode(mode, cx);
                                });
                            }
                        },
                    )),
            );

        let content = v_flex()
            .w_full()
            .min_w(px(0.0))
            .max_w(space(ShellMetrics::CONTENT_MAX_WIDTH))
            .mx_auto()
            .gap_6()
            .pb(space(112.0))
            .when(!package_summaries.is_empty(), |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .min_w(px(0.0))
                        .gap_2()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .child(page_section_title(i18n::string("snippets.page.packages")))
                        .child(
                            card_grid(available_width)
                                .id("snippet-packages-grid")
                                .test_support()
                                .children(package_summaries.into_iter().map(|(package, count)| {
                                    let package_name = package.clone();
                                    let is_selected =
                                        selected_package_filter.is_some_and(|selected| {
                                            selected.eq_ignore_ascii_case(package_name.as_str())
                                        });
                                    snippet_package_card(package, count, is_selected, {
                                        let controller = controller.clone();
                                        move |_, cx| {
                                            let package_name = package_name.clone();
                                            controller.update(cx, |controller, cx| {
                                                controller.toggle_snippets_package_filter(
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
                    .child(if visible_snippets.is_empty() {
                        snippets_empty_state(i18n::string("snippets.empty.no_package_matches"))
                            .into_any_element()
                    } else {
                        animated_card_collection(
                            "snippets-grid",
                            catalog_view.snippets_view_mode,
                            card_grid_columns(available_width),
                            8.0,
                            visible_snippets
                                .into_iter()
                                .map(|snippet| {
                                    let on_open_existing = on_open_existing.clone();
                                    let snippet_id = snippet.id.clone();
                                    let layout_id = format!("snippet-layout-{snippet_id}").into();
                                    let on_open = move |window: &mut Window, cx: &mut App| {
                                        on_open_existing(snippet_id.clone(), window, cx);
                                    };
                                    let item = if is_list {
                                        snippet_list_row(snippet, on_open).into_any_element()
                                    } else {
                                        snippet_command_card(snippet, on_open).into_any_element()
                                    };
                                    (layout_id, item)
                                })
                                .collect(),
                        )
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
                            .id("snippets-collection-scroll")
                            .test_support()
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .child(div().size_full().overflow_y_scrollbar().child(
                                div().px(space(ShellMetrics::CATALOG_INSET)).child(content),
                            )),
                    ),
            )
            .into_any_element()
    }

    /// The page's create command, docked into the bottom navigation bar.
    pub(in crate::ui) fn render_snippets_dock_action(
        &self,
        cx: &App,
        on_open: impl Fn(&mut Window, &mut App) + 'static,
    ) -> gpui_kit::AnyElement {
        dock_action_button(
            "snippets-create-fab",
            i18n::string("snippets.editor.create_title"),
            cx,
            on_open,
        )
        .into_any_element()
    }

    pub(in crate::ui) fn render_snippets_editor_sidebar(
        &self,
        controller: Entity<Self>,
        on_save: impl Fn(&mut Window, &mut App) + 'static,
        cx: &App,
    ) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let snippets = self.snippets().clone();
        let is_editing = self
            .selected_snippet()
            .and_then(|index| snippets.get(index))
            .is_some();
        let available_packages = Self::collect_available_snippet_packages(&snippets);
        let forms = self.snippets_forms();
        let editor_title = if is_editing {
            i18n::string("snippets.editor.edit_title")
        } else {
            i18n::string("snippets.editor.create_title")
        };
        let header = h_flex().w_full().items_start().gap_4().child(
            v_flex().flex_1().gap_1().child(
                div()
                    .text_size(miaominal_settings::FontSize::PageTitle.scaled())
                    .text_color(rgb(roles.on_surface))
                    .child(editor_title),
            ),
        );

        let mut footer_actions = Vec::new();
        if is_editing {
            footer_actions.push(
                editor_footer_button(
                    "snippets-editor-delete",
                    i18n::string("dialogs.snippet_delete.confirm"),
                    Some(AppIcon::Trash),
                    EditorFooterActionTone::Destructive,
                    cx,
                    {
                        let controller = controller.clone();
                        move |_, cx| {
                            controller.update(cx, |controller, cx| {
                                controller.request_selected_snippet_delete(cx);
                            });
                        }
                    },
                )
                .into_any_element(),
            );
        }
        footer_actions.push(
            editor_footer_button(
                "snippets-editor-cancel",
                i18n::string("snippets.editor.cancel"),
                Some(AppIcon::Close),
                EditorFooterActionTone::Text,
                cx,
                {
                    let controller = controller.clone();
                    move |_, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.close_snippets_editor(cx);
                        });
                    }
                },
            )
            .into_any_element(),
        );
        footer_actions.push(
            editor_footer_button(
                "snippets-editor-save",
                i18n::string("snippets.editor.save"),
                Some(AppIcon::Check),
                EditorFooterActionTone::Primary,
                cx,
                on_save,
            )
            .into_any_element(),
        );
        let footer = editor_footer_actions(footer_actions);

        div()
            .id("snippets-editor-sidebar")
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
                                    .child(SectionCard::new(
                                        AppIcon::Notebook,
                                        i18n::string("snippets.editor.general"),
                                        v_flex()
                                            .w_full()
                                            .gap_4()
                                            .child(
                                                div()
                                                    .text_size(miaominal_settings::FontSize::Input.scaled())
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
                                                                md3_select(&forms.package_select)
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
                                                                    let controller = controller.clone();
                                                                    move |window, cx| {
                                                                        controller.update(cx, |controller, cx| {
                                                                            controller.begin_new_snippet_package(
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
