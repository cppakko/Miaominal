use crate::ui::assets::AppIcon;
use crate::ui::components::empty_state::{EmptyState, EmptyStateSize, empty_state_tonal_action};
use crate::ui::components::group_accent::group_accent_palette;
use crate::ui::components::shell_tokens::{ShellMetrics, space};
use crate::ui::components::{
    SearchInputStyle, TextInputSurface, card_surface, icon_button, search_filter_input,
    surface_text_input_stack,
};
use crate::ui::contracts::TabId;
use crate::ui::contracts::session::SessionTerminalPort;
use crate::ui::features::snippets::SnippetsController;
use crate::ui::i18n;
use crate::ui::utils::truncate_with_ellipsis;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, MouseButton, SharedString, Window, div, px, rgb};

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
    let item_id = SharedString::from(format!("session-snippet-package-card-{}", title.as_str()));
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
    .id(item_id)
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

impl SnippetsController {
    pub(in crate::ui) fn render_workspace_panel(
        &self,
        controller: Entity<Self>,
        terminal_port: SessionTerminalPort,
        terminal_tab_id: Option<TabId>,
        has_floating_menu: bool,
        cx: &App,
    ) -> gpui_kit::AnyElement {
        let snippets_filter_input = self.workspace_filter_input.clone();
        let selected_package_filter = self.workspace_package_filter.borrow().clone();
        let empty_for_package = controller.clone();
        let owner = controller.clone();
        let on_package_toggle = move |package, cx: &mut App| {
            controller.update(cx, |owner, cx| owner.toggle_workspace_package(package, cx));
        };
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let filter_text = snippets_filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let snippets = self.snippets();
        let search_matched_snippets: Vec<_> = snippets
            .iter()
            .filter(|snippet| miaominal_core::snippet::matches_filter(snippet, &filter_text))
            .cloned()
            .collect();
        let mut package_summaries: Vec<_> =
            SnippetsController::collect_available_snippet_packages(&search_matched_snippets)
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
        let selected_package_filter = selected_package_filter.as_deref().filter(|selected| {
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

        let empty_results = visible_snippets.is_empty();
        let content = if empty_results {
            let state = if snippets.is_empty() {
                EmptyState::new(
                    AppIcon::Notebook,
                    i18n::string("workspace.panel.snippets.empty_title"),
                )
                .description(i18n::string("workspace.panel.snippets.empty"))
                .size(EmptyStateSize::Panel)
            } else if search_matched_snippets.is_empty() {
                EmptyState::new(
                    AppIcon::Search,
                    i18n::string("workspace.panel.snippets.no_search_matches_title"),
                )
                .description(i18n::string("workspace.panel.snippets.no_search_matches"))
                .size(EmptyStateSize::Panel)
            } else {
                EmptyState::new(
                    AppIcon::Filter,
                    i18n::string("snippets.empty.no_package_matches_title"),
                )
                .description(i18n::string("snippets.empty.no_package_matches"))
                .size(EmptyStateSize::Panel)
                .action(empty_state_tonal_action(
                    "workspace-snippets-clear-package",
                    Some(AppIcon::Rotate),
                    i18n::string("workspace.panel.snippets.clear_package_filter"),
                    cx,
                    move |_, cx| {
                        empty_for_package.update(cx, |owner, cx| {
                            *owner.workspace_package_filter.borrow_mut() = None;
                            cx.notify();
                        });
                    },
                ))
            };
            state.build_page().into_any_element()
        } else {
            let mut list = v_flex().gap_2();
            for snippet in visible_snippets {
                let terminal_port = terminal_port.clone();
                let run_owner = owner.clone();
                let description = snippet.description.clone();
                let script = snippet.script.clone();
                let preview_line = snippet
                    .script
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or(snippet.script.as_str())
                    .trim();
                let preview = truncate_with_ellipsis(preview_line, 48);
                let card_id =
                    SharedString::from(format!("session-snippet-card-{}", snippet.id.as_str()));
                let button_id =
                    SharedString::from(format!("session-snippet-send-{}", snippet.id.as_str()));

                list = list.child(
                    div()
                        .id(card_id)
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
                                        move |window, cx| {
                                            let description = description.clone();
                                            let script = script.clone();
                                            let terminal_port = terminal_port.clone();
                                            if let Some(tab_id) = terminal_tab_id {
                                                run_owner.update(cx, |controller, cx| {
                                                    controller.trigger_workspace_snippet(
                                                        description,
                                                        script,
                                                        tab_id,
                                                        terminal_port,
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }
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
                            .child(v_flex().w_full().gap_2().children(
                                package_summaries.into_iter().map(|(package, count)| {
                                    let package_name = package.clone();
                                    let is_selected =
                                        selected_package_filter.is_some_and(|selected| {
                                            selected.eq_ignore_ascii_case(package_name.as_str())
                                        });
                                    session_snippet_package_card(package, count, is_selected, {
                                        let on_package_toggle = on_package_toggle.clone();
                                        move |_, cx| {
                                            let package_name = package_name.clone();
                                            on_package_toggle(package_name.clone(), cx);
                                        }
                                    })
                                }),
                            )),
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

        let pending_variable_card = self.pending_variable_run.borrow().clone().map(|pending| {
            let cancel_owner = owner.clone();
            let confirm_owner = owner.clone();
            card_surface(roles.surface_container_high, 14.0)
                .id("session-snippet-variables-card")
                .w_full()
                .p_3()
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(rgb(roles.on_surface))
                                .child(i18n::string_args(
                                    "snippets.variables.title",
                                    &[("name", pending.description.as_str())],
                                )),
                        )
                        .children(pending.fields.iter().map(|(variable, input)| {
                            surface_text_input_stack(
                                variable.name.clone(),
                                input.clone(),
                                TextInputSurface::Low,
                                false,
                            )
                        }))
                        .child(
                            h_flex()
                                .w_full()
                                .justify_end()
                                .gap_2()
                                .child(
                                    Button::new("snippet-variables-cancel")
                                        .ghost()
                                        .small()
                                        .label(i18n::string("snippets.editor.cancel"))
                                        .on_click(move |_, _, cx| {
                                            cancel_owner.update(cx, |controller, cx| {
                                                controller.cancel_snippet_variable_run(cx);
                                            });
                                        }),
                                )
                                .child(
                                    Button::new("snippet-variables-run")
                                        .primary()
                                        .small()
                                        .label(i18n::string("snippets.variables.run"))
                                        .on_click(move |_, _, cx| {
                                            confirm_owner.update(cx, |controller, cx| {
                                                controller.confirm_snippet_variable_run(cx);
                                            });
                                        }),
                                ),
                        ),
                )
        });

        v_flex()
            .id("session-snippets-panel-content")
            .size_full()
            .gap_3()
            .overflow_hidden()
            .p_3()
            .when_some(pending_variable_card, |this, card| this.child(card))
            .when(!snippets.is_empty(), |this| {
                this.child(search_filter_input(
                    &snippets_filter_input,
                    SearchInputStyle::Compact,
                    None,
                ))
            })
            .child(div().flex_1().min_h_0().child(if empty_results {
                div()
                    .size_full()
                    .when(has_floating_menu, |this| {
                        this.pb(space(ShellMetrics::SESSION_PANEL_MENU_CLEARANCE))
                    })
                    .child(content)
                    .into_any_element()
            } else {
                div()
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .w_full()
                            .when(has_floating_menu, |this| {
                                this.pb(space(ShellMetrics::SESSION_PANEL_MENU_CLEARANCE))
                            })
                            .child(content),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    }
}
