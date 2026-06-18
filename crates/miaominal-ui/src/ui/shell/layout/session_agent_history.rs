use super::super::*;
use super::session_agent_composer;
use super::session_agent_utils::format_relative_chat_time;
use gpui::{Size, size};
use gpui_component::v_virtual_list;
use std::rc::Rc;

pub(in crate::ui::shell::layout) fn render_session_agent_history_panel(
    app: &AppView,
    entity: Entity<AppView>,
    _window: &mut Window,
    _cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let sessions = app.data.chat_sessions.clone();
    let current_session_id = app.session_agent.session_id.clone();
    let history_scroll_handle = app
        .workspace_state
        .session_agent_history_scroll_handle
        .clone();

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
                .child(
                    h_flex().w_full().h(px(30.0)).items_center().gap_2().child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(miaominal_settings::FontSize::Subheading.scaled())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(roles.on_surface))
                            .child("AI Chat"),
                    ),
                )
                .child(if sessions.is_empty() {
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
                        .child("No saved chats")
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .relative()
                        .child({
                            let item_sizes: Rc<Vec<Size<Pixels>>> = Rc::new(
                                (0..sessions.len())
                                    .map(|_| size(px(0.0), px(58.0)))
                                    .collect(),
                            );
                            let view_entity = entity.clone();
                            let current_sid = current_session_id.clone();
                            let session_count = sessions.len();

                            v_virtual_list(
                                entity.clone(),
                                "session-agent-history-list",
                                item_sizes,
                                move |this, visible_range, _window, _cx| {
                                    let material = miaominal_settings::current_theme().material;
                                    let roles = material.roles;
                                    let text_muted = crate::ui::theme::palette_tone_rgb(
                                        material.palettes.neutral_variant,
                                        if material.dark { 65 } else { 50 },
                                    );
                                    let view_entity = view_entity.clone();
                                    let current_sid = current_sid.clone();

                                    visible_range
                                        .filter(|ix| *ix < session_count)
                                        .map(|ix| {
                                            let session = &this.data.chat_sessions[ix];
                                            let open_entity = view_entity.clone();
                                            let delete_entity = view_entity.clone();
                                            let session_id = session.id.clone();
                                            let delete_session_id = session.id.clone();
                                            let is_current = current_sid.as_deref()
                                                == Some(session.id.as_str());
                                            let is_busy =
                                                this.session_agent_session_is_busy(&session.id);
                                            let title = if session.title.trim().is_empty() {
                                                if is_current {
                                                    "Current chat".to_string()
                                                } else {
                                                    "Untitled chat".to_string()
                                                }
                                            } else {
                                                session.title.clone()
                                            };
                                            let delete_title = title.clone();
                                            let updated_at =
                                                format_relative_chat_time(session.updated_at);
                                            let status_label = if is_busy {
                                                Some("Working")
                                            } else if is_current {
                                                Some("Current")
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
                                                        this.load_session_agent_chat(session_id, cx);
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
                                                            .bg(rgb(if is_busy {
                                                                roles.primary
                                                            } else {
                                                                roles.surface_container_highest
                                                            }))
                                                            .text_size(
                                                                miaominal_settings::FontSize::Body
                                                                    .scaled(),
                                                            )
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(rgb(if is_busy {
                                                                roles.on_primary
                                                            } else {
                                                                roles.on_surface_variant
                                                            }))
                                                            .child(label),
                                                    )
                                                })
                                                .child(icon_button(
                                                    AppIcon::Trash,
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
                                                        entity.update(cx, |this, cx| {
                                                            this.request_session_agent_chat_delete(
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
        .child(session_agent_composer::render_session_agent_composer(app, entity.clone()))
        .into_any_element()
}
