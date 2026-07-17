use super::super::*;
use super::session_agent_composer;
use super::session_agent_utils::format_relative_chat_time;
use crate::ui::components::icon_button_with_tooltip;
use crate::ui::i18n;
use gpui::AnimationExt as _;
use gpui::{Size, size};
use gpui_component::v_virtual_list;
use std::rc::Rc;

pub(in crate::ui::shell::layout) fn render_session_agent_history_panel(
    controller: &AgentController,
    agent: Entity<AgentController>,
    settings: Entity<SettingsController>,
    _window: &mut Window,
    cx: &mut App,
    search_visibility: Option<f32>,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let all_sessions = Rc::new(controller.chat_sessions().to_vec());
    let current_session_id = controller.session_agent().session_id.clone();
    let runtime_statuses = Rc::new(
        all_sessions
            .iter()
            .map(|session| {
                if current_session_id.as_deref() == Some(session.id.as_str()) {
                    (
                        controller.session_agent().is_busy(),
                        controller
                            .session_agent()
                            .has_tool_call_waiting_for_confirmation(),
                    )
                } else {
                    (
                        controller.background_session_is_busy(&session.id),
                        controller.background_session_needs_approval(&session.id),
                    )
                }
            })
            .collect::<Vec<_>>(),
    );
    let history_scroll_handle = controller.history_scroll_handle();
    let search_filter_input_entity = controller.session_filter_input();

    // Filter sessions by search query — store indices into all_sessions
    let search_query = controller.session_agent().search_query.clone();
    let filtered_indices: Vec<usize> = if let Some(ref query) = search_query {
        let query_lower = query.to_lowercase();
        all_sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.title.to_lowercase().contains(&query_lower))
            .map(|(i, _)| i)
            .collect()
    } else {
        (0..all_sessions.len()).collect()
    };

    let has_sessions = !all_sessions.is_empty();

    v_flex()
        .id("session-agent-history-panel")
        .size_full()
        .overflow_hidden()
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .px_3()
                .pt_2()
                .gap_3()
                // Search bar
                .when_some(search_visibility, move |this, visibility| {
                    this.child(
                        div()
                            .w_full()
                            .opacity(visibility)
                            .top(px((1.0 - visibility) * 8.0))
                            .child(search_filter_input(
                                &search_filter_input_entity.clone(),
                                SearchInputStyle::Compact,
                                None,
                            )),
                    )
                })
                .child(if !has_sessions {
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_h(px(160.0))
                        .w_full()
                        .items_center()
                        .justify_center()
                        .flex()
                        .text_center()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .text_color(rgb(text_muted))
                        .child(i18n::string("workspace.panel.agent.history.empty"))
                        .into_any_element()
                } else if filtered_indices.is_empty() {
                    // Empty state: no matching sessions
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_h(px(160.0))
                        .w_full()
                        .items_center()
                        .justify_center()
                        .flex()
                        .text_center()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .text_color(rgb(text_muted))
                        .child(i18n::string("search.messages.no_matches"))
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .relative()
                        .child({
                            let filtered_count = filtered_indices.len();
                            let filtered_indices_rc: Rc<Vec<usize>> = Rc::new(filtered_indices);
                            let item_sizes: Rc<Vec<Size<Pixels>>> = Rc::new(
                                (0..filtered_count)
                                    .map(|_| size(px(0.0), px(58.0)))
                                    .collect(),
                            );
                            let view_entity = agent.clone();
                            let agent_entity = agent.clone();
                            let current_sid = current_session_id.clone();
                            let session_count = filtered_count;
                            let filtered_ix = filtered_indices_rc.clone();
                            let sessions = all_sessions.clone();
                            let statuses = runtime_statuses.clone();

                            v_virtual_list(
                                agent.clone(),
                                "session-agent-history-list",
                                item_sizes,
                                move |_this, visible_range, _window, _cx| {
                                    let material = miaominal_settings::current_theme().material;
                                    let roles = material.roles;
                                    let text_muted = crate::ui::theme::palette_tone_rgb(
                                        material.palettes.neutral_variant,
                                        if material.dark { 65 } else { 50 },
                                    );
                                    let warning_color = material.extended.warning.color;
                                    let warning_on_color = material.extended.warning.on_color;
                                    let view_entity = view_entity.clone();
                                    let agent_entity = agent_entity.clone();
                                    let current_sid = current_sid.clone();
                                    let filtered_ix = filtered_ix.clone();
                                    let sessions = sessions.clone();
                                    let statuses = statuses.clone();

                                    visible_range
                                        .filter(|ix| *ix < session_count)
                                        .map(|ix| {
                                            // Map virtual list index to actual session index
                                            let actual_ix = filtered_ix[ix];
                                            let session = &sessions[actual_ix];
                                            let (is_busy, needs_approval) = statuses[actual_ix];
                                            let open_entity = view_entity.clone();
                                            let delete_entity = agent_entity.clone();
                                            let rename_entity = agent_entity.clone();
                                            let session_id = session.id.clone();
                                            let delete_session_id = session.id.clone();
                                            let rename_session_id = session.id.clone();
                                            let is_current = current_sid.as_deref()
                                                == Some(session.id.as_str());
                                            let title = if session.title.trim().is_empty() {
                                                if is_current {
                                                    i18n::string("workspace.panel.agent.history.current_chat")
                                                } else {
                                                    i18n::string("workspace.panel.agent.history.untitled_chat")
                                                }
                                            } else {
                                                session.title.clone()
                                            };
                                            let delete_title = title.clone();
                                            let rename_title = title.clone();
                                            let updated_at =
                                                format_relative_chat_time(session.updated_at);
                                            let status_label = if needs_approval {
                                                Some(i18n::string("workspace.panel.agent.history.needs_approval"))
                                            } else if is_busy {
                                                Some(i18n::string("workspace.panel.agent.history.working"))
                                            } else if is_current {
                                                Some(i18n::string("workspace.panel.agent.history.current"))
                                            } else {
                                                None
                                            };

                                            h_flex()
                                                .id(SharedString::from(format!(
                                                    "chat-session-row-{}",
                                                    session.id
                                                )))
                                                .w_full()
                                                .min_h(px(58.0))
                                                .items_center()
                                                .gap_2()
                                                .rounded(px(8.0))
                                                .bg(rgb(if is_current {
                                                    roles.secondary_container
                                                } else {
                                                    roles.surface_container_high
                                                }))
                                                .pl_2()
                                                .pr(px(20.0))
                                                .py_2()
                                                .cursor_pointer()
                                                .hover(move |this| {
                                                    this.bg(rgb(if is_current {
                                                        roles.secondary_container
                                                    } else {
                                                        roles.surface_container_highest
                                                    }))
                                                })
                                                .on_click(move |_click, _window, cx| {
                                                    let entity = open_entity.clone();
                                                    let session_id = session_id.clone();
                                                    entity.update(cx, |this, cx| {
                                                        this.finish_text_drag(cx);
                                                        this.open_chat_session(session_id, cx);
                                                    });
                                                })
                                                .child(
                                                    v_flex()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .w_full()
                                                                .overflow_hidden()
                                                                .text_ellipsis()
                                                                .text_size(
                                                                    miaominal_settings::FontSize::Input
                                                                        .scaled(),
                                                                )
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_color(rgb(if is_current {
                                                                    roles.on_secondary_container
                                                                } else {
                                                                    roles.on_surface
                                                                }))
                                                                .child(title.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(
                                                                    miaominal_settings::FontSize::Body
                                                                        .scaled(),
                                                                )
                                                                .text_color(rgb(text_muted))
                                                                .child(updated_at),
                                                        ),
                                                )
                                                .when_some(status_label, |this, label| {
                                                    this.child(
                                                        div()
                                                            .flex_shrink_0()
                                                            .rounded(px(999.0))
                                                            .px_2()
                                                            .py_1()
                                                            .bg(rgb(if needs_approval {
                                                                warning_color
                                                            } else if is_busy {
                                                                roles.primary
                                                            } else {
                                                                roles.surface_container_highest
                                                            }))
                                                            .text_size(
                                                                miaominal_settings::FontSize::Body
                                                                    .scaled(),
                                                            )
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(rgb(if needs_approval {
                                                                warning_on_color
                                                            } else if is_busy {
                                                                roles.on_primary
                                                            } else {
                                                                roles.on_surface_variant
                                                            }))
                                                            .child(label),
                                                    )
                                                })
                                                .child(icon_button_with_tooltip(
                                                    AppIcon::Edit,
                                                    i18n::string("workspace.panel.agent.tooltips.rename_chat"),
                                                    24.0,
                                                    8.0,
                                                    Some(roles.surface_container_high),
                                                    Some(text_muted),
                                                    None,
                                                    move |window, cx| {
                                                        cx.stop_propagation();
                                                        let entity = rename_entity.clone();
                                                        let session_id = rename_session_id.clone();
                                                        let title = rename_title.clone();
                                                        entity.update(cx, |controller, cx| {
                                                            controller.request_chat_session_rename(
                                                                session_id, title, window, cx,
                                                            );
                                                        });
                                                    },
                                                ))
                                                .child(icon_button_with_tooltip(
                                                    AppIcon::Trash,
                                                    i18n::string("workspace.panel.agent.tooltips.delete_chat"),
                                                    24.0,
                                                    8.0,
                                                    Some(roles.surface_container_high),
                                                    Some(text_muted),
                                                    None,
                                                    move |_window, cx| {
                                                        cx.stop_propagation();
                                                        let entity = delete_entity.clone();
                                                        let session_id = delete_session_id.clone();
                                                        let title = delete_title.clone();
                                                        entity.update(cx, |controller, cx| {
                                                            controller.request_chat_session_delete(
                                                                session_id, title, cx,
                                                            );
                                                        });
                                                    },
                                                ))
                                                .into_any_element()
                                        })
                                        .collect::<Vec<_>>()
                                },
                            )
                            .track_scroll(&history_scroll_handle)
                            .gap_2()
                            .overflow_x_hidden()
                        })
                        .vertical_scrollbar(&history_scroll_handle)
                        .into_any_element()
                }),
        )
        .child(session_agent_composer::render_session_agent_composer(
            controller,
            agent,
            settings,
            cx,
        ))
        .with_animation(
            "session-agent-history-view",
            container_transition_animation(),
            |element, delta| element.opacity(delta).top(px((1.0 - delta) * 8.0)),
        )
        .into_any_element()
}
