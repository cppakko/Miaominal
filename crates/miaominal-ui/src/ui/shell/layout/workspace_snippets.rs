use crate::ui::i18n;

use super::super::pages::shell_empty_state;
use super::super::*;

fn session_snippet_package_card(
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
            roles.surface_container_high
        },
        16.0,
    )
    .w_full()
    .cursor_pointer()
    .p_3()
    .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, cx| {
        on_click(window, cx);
    })
    .child(
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                h_flex()
                    .min_w(px(0.0))
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(36.0))
                            .rounded(px(12.0))
                            .bg(rgb(if is_selected {
                                palette.accent
                            } else {
                                roles.surface_container_low
                            }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(miaominal_settings::FontSize::Body.scaled())
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
                                    .text_size(miaominal_settings::FontSize::Input.scaled())
                                    .text_color(rgb(roles.on_surface))
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .text_color(rgb(text_muted))
                                    .child(count_label),
                            ),
                    ),
            ),
    )
}

impl AppView {
    pub(in crate::ui::shell::layout) fn render_session_snippets_panel(
        &self,
        entity: Entity<Self>,
        cx: &App,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let filter_text = self
            .workspace_forms
            .snippets_panel
            .filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let search_matched_snippets: Vec<_> = self
            .data
            .snippets
            .iter()
            .filter(|snippet| miaominal_core::snippet::matches_filter(snippet, &filter_text))
            .cloned()
            .collect();
        let mut package_summaries: Vec<_> =
            Self::collect_available_snippet_packages(&search_matched_snippets)
                .into_iter()
                .map(|package| {
                    let count = search_matched_snippets
                        .iter()
                        .filter(|snippet| snippet.package.eq_ignore_ascii_case(package.as_str()))
                        .count();
                    (package, count)
                })
                .collect();
        package_summaries.sort_by(|left, right| {
            left.0
                .to_ascii_lowercase()
                .cmp(&right.0.to_ascii_lowercase())
        });
        let selected_package_filter = self
            .workspace_forms
            .snippets_panel
            .selected_package_filter
            .as_deref()
            .filter(|selected| {
                package_summaries
                    .iter()
                    .any(|(package, _)| package.eq_ignore_ascii_case(selected))
            });
        let mut visible_snippets: Vec<_> = search_matched_snippets
            .iter()
            .filter(|snippet| {
                selected_package_filter
                    .is_none_or(|package| snippet.package.eq_ignore_ascii_case(package))
            })
            .cloned()
            .collect();
        visible_snippets.sort_by(|left, right| {
            left.description
                .to_ascii_lowercase()
                .cmp(&right.description.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });

        let content = if self.data.snippets.is_empty() {
            shell_empty_state(
                AppIcon::Notebook,
                i18n::string("workspace.panel.snippets.empty"),
            )
            .into_any_element()
        } else if search_matched_snippets.is_empty() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(text_muted))
                        .child(i18n::string("workspace.panel.snippets.no_search_matches")),
                )
                .into_any_element()
        } else if visible_snippets.is_empty() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(text_muted))
                        .child(i18n::string("snippets.empty.no_package_matches")),
                )
                .into_any_element()
        } else {
            let mut list = v_flex().gap_2();
            for snippet in visible_snippets {
                let send_entity = entity.clone();
                let script = snippet.script.clone();
                let preview_line = snippet
                    .script
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or(snippet.script.as_str())
                    .trim();
                let preview = truncate_with_ellipsis(preview_line, 48);
                let button_id = SharedString::from(format!("session-snippet-send-{}", snippet.id));

                list = list.child(
                    div()
                        .w_full()
                        .rounded(px(14.0))
                        .bg(rgb(roles.surface))
                        .p_3()
                        .child(
                            v_flex().gap_2().child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_size(
                                                        miaominal_settings::FontSize::Body.scaled(),
                                                    )
                                                    .text_color(rgb(roles.on_surface))
                                                    .child(snippet.description.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(
                                                        miaominal_settings::FontSize::Body.scaled(),
                                                    )
                                                    .text_color(rgb(text_muted))
                                                    .child(preview),
                                            ),
                                    )
                                    .child(div().id(button_id).child(icon_button(
                                        AppIcon::Play,
                                        36.0,
                                        12.0,
                                        Some(roles.primary),
                                        Some(roles.on_primary),
                                        None,
                                        move |_window, cx| {
                                            let script = script.clone();
                                            send_entity.update(cx, |this, cx| {
                                                this.send_paste_text(script.clone(), cx);
                                            });
                                        },
                                    ))),
                            ),
                        ),
                );
            }
            v_flex()
                .w_full()
                .gap_3()
                .when(!package_summaries.is_empty(), |this| {
                    this.child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .text_color(rgb(roles.on_surface))
                                    .child(i18n::string("snippets.page.packages")),
                            )
                            .child(
                                v_flex().w_full().gap_2().children(
                                    package_summaries.into_iter().map(|(package, count)| {
                                        let package_name = package.clone();
                                        let is_selected = selected_package_filter
                                            .is_some_and(|selected| {
                                                selected.eq_ignore_ascii_case(package_name.as_str())
                                            });
                                        session_snippet_package_card(
                                            package,
                                            count,
                                            is_selected,
                                            {
                                                let entity = entity.clone();
                                                move |_, cx| {
                                                    let package_name = package_name.clone();
                                                    entity.update(cx, |this, cx| {
                                                        this.handle_workspace_snippets_package_filter_toggle(
                                                            package_name.clone(),
                                                            cx,
                                                        );
                                                    });
                                                }
                                            },
                                        )
                                    }),
                                ),
                            ),
                    )
                })
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface))
                                .child(i18n::string("snippets.page.snippets")),
                        )
                        .child(list),
                )
                .into_any_element()
        };

        v_flex()
            .id("session-snippets-panel-content")
            .size_full()
            .gap_3()
            .overflow_hidden()
            .p_3()
            .when(!self.data.snippets.is_empty(), |this| {
                this.child(search_filter_input(
                    &self.workspace_forms.snippets_panel.filter_input,
                    SearchInputStyle::Compact,
                    None,
                ))
            })
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .child(content),
            )
            .into_any_element()
    }
}
