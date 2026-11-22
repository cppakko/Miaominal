use super::super::*;
use crate::ui::i18n;
use gpui_kit::AnimationExt as _;

pub(in crate::ui::features::agent) fn render_session_agent_at_mention_popup(
    controller: Entity<AgentController>,
    candidates: Vec<SessionAgentTargetCandidate>,
    query: String,
    selected_index: usize,
) -> gpui_kit::AnyElement {
    div()
        .id("agent-at-mention-popup")
        .w_full()
        .min_h(px(96.0))
        .max_h(px(360.0))
        .overflow_y_scroll()
        .occlude()
        .child(render_session_agent_at_mention_menu(
            controller,
            candidates,
            query,
            selected_index,
        ))
        .with_animation(
            "session-agent-at-mention-popup",
            overlay_enter_animation(),
            |element, delta| element.opacity(delta).top(px((1.0 - delta) * 8.0)),
        )
        .into_any_element()
}

pub(in crate::ui::features::agent) fn render_session_agent_at_mention_menu(
    controller: Entity<AgentController>,
    candidates: Vec<SessionAgentTargetCandidate>,
    query: String,
    selected_index: usize,
) -> gpui_kit::AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let filtered = super::super::controller::filter_at_mention_candidates(candidates, &query);
    let selected_index = (!filtered.is_empty()).then(|| selected_index.min(filtered.len() - 1));

    v_flex()
        .w_full()
        .gap_1()
        .rounded(px(8.0))
        .bg(rgb(roles.surface_container_lowest))
        .p_1()
        .when(filtered.is_empty(), |this| {
            this.child(
                crate::ui::components::empty_state::EmptyState::new(
                    AppIcon::Computer,
                    i18n::string("workspace.panel.agent.targets.empty"),
                )
                .size(crate::ui::components::empty_state::EmptyStateSize::Inline)
                .build(),
            )
        })
        .children(filtered.into_iter().enumerate().map(|(ix, candidate)| {
            let name = candidate.name.clone();
            let id_name = candidate.name.clone();
            let click_controller = controller.clone();
            let is_selected = selected_index == Some(ix);
            h_flex()
                .id(SharedString::from(format!(
                    "agent-at-mention-row-{id_name}"
                )))
                .w_full()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .bg(rgb(if is_selected {
                    roles.secondary_container
                } else {
                    roles.surface_container_low
                }))
                .cursor_pointer()
                .hover(move |this| this.bg(rgb(roles.secondary_container)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .on_mouse_up(MouseButton::Left, move |_, window, cx| {
                    let name = name.clone();
                    let controller = click_controller.clone();
                    controller.update(cx, |controller, cx| {
                        controller.insert_at_mention(name, window, cx);
                    });
                    cx.stop_propagation();
                })
                .child(
                    div().flex_1().min_w_0().overflow_hidden().child(
                        v_flex()
                            .gap(px(1.0))
                            .child(
                                div()
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(roles.on_surface))
                                    .child(format!("@{}", candidate.name)),
                            )
                            .child(
                                div()
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .text_color(rgb(roles.on_surface_variant))
                                    .child(candidate.detail),
                            ),
                    ),
                )
                .when(!candidate.resolved, |this| {
                    this.child(
                        div()
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .text_color(rgb(roles.error))
                            .child(i18n::string("workspace.panel.agent.targets.offline")),
                    )
                })
                .with_animation(
                    SharedString::from(format!("agent-at-mention-row-enter-{id_name}")),
                    list_enter_animation(),
                    |element, delta| element.opacity(delta).top(px((1.0 - delta) * 6.0)),
                )
                .into_any_element()
        }))
        .into_any_element()
}
