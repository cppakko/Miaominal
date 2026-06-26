use super::super::*;
use super::session_agent_panel::clamp_session_agent_panel_width;
use crate::ui::i18n;
use gpui::AnimationExt as _;

pub(in crate::ui::shell::layout) fn render_session_agent_at_mention_overlay(
    app: &AppView,
    entity: Entity<AppView>,
    query: String,
) -> gpui::AnyElement {
    let agent_panel_width =
        clamp_session_agent_panel_width(app.workspace_state.session_agent_panel_width);
    let popup_width = (agent_panel_width - 16.0).max(200.0);
    let candidates = app.session_agent_target_candidates();

    div()
        .id("agent-at-mention-overlay")
        .absolute()
        .right(px(8.0))
        .bottom(px(STATUS_BAR_HEIGHT + 154.0))
        .w(px(popup_width))
        .occlude()
        .child(render_session_agent_at_mention_menu(
            entity, candidates, query,
        ))
        .with_animation(
            "session-agent-at-mention-popup",
            overlay_enter_animation(),
            |element, delta| element.opacity(delta).top(px((1.0 - delta) * 8.0)),
        )
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_session_agent_at_mention_menu(
    entity: Entity<AppView>,
    candidates: Vec<SessionAgentTargetCandidate>,
    query: String,
) -> gpui::AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let query = query.trim().to_ascii_lowercase();
    let filtered = candidates
        .into_iter()
        .filter(|candidate| {
            query.is_empty() || candidate.name.to_ascii_lowercase().contains(&query)
        })
        .take(8)
        .collect::<Vec<_>>();

    v_flex()
        .w_full()
        .min_h(px(96.0))
        .max_h(px(306.0))
        .overflow_y_scrollbar()
        .rounded(px(8.0))
        .bg(rgb(roles.surface_container_lowest))
        .p_1()
        .when(filtered.is_empty(), |this| {
            this.child(
                div()
                    .w_full()
                    .px_3()
                    .py_2()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string("workspace.panel.agent.targets.empty")),
            )
        })
        .children(filtered.into_iter().map(|candidate| {
            let name = candidate.name.clone();
            let id_name = candidate.name.clone();
            let click_entity = entity.clone();
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
                .bg(rgb(roles.surface_container_low))
                .cursor_pointer()
                .hover(move |this| this.bg(rgb(roles.secondary_container)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .on_mouse_up(MouseButton::Left, move |_, window, cx| {
                    let name = name.clone();
                    let entity = click_entity.clone();
                    entity.update(cx, |this, cx| {
                        this.insert_session_agent_at_mention(name, window, cx);
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
