use super::super::*;
use super::session_agent_utils::*;
use crate::ui::components::icon_button_with_tooltip;
use crate::ui::i18n;
use gpui::{Animation, AnimationExt as _};
use std::time::Duration;

const SESSION_AGENT_SEND_PULSE_DURATION: Duration = Duration::from_millis(1100);

pub(in crate::ui::shell::layout) fn render_session_agent_composer(
    app: &AppView,
    entity: Entity<AppView>,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let provider_select = app.panel_forms.settings.ai_provider_select.clone();
    let prompt_input = app.workspace_forms.agent.prompt_input.clone();
    let prompt_menu_input = prompt_input.clone();
    let pty_toggle_entity = entity.clone();
    let send_entity = entity;
    let waiting = app.session_agent.is_busy();

    div()
        .flex_shrink_0()
        .p_2()
        .relative()
        .child(
            v_flex()
                .w_full()
                .gap_2()
                .rounded(px(8.0))
                .bg(rgb(roles.surface_container_high))
                .p_2()
                .child(app.render_session_agent_target_chips(pty_toggle_entity.clone()))
                .child(
                    div()
                        .w_full()
                        .min_h(px(86.0))
                        .max_h(px(190.0))
                        .rounded(px(6.0))
                        .relative()
                        .overflow_hidden()
                        .id("session-agent-prompt-input-menu")
                        .child(
                            Input::new(&prompt_input)
                                .w_full()
                                .appearance(false)
                                .focus_bordered(false)
                                .p_1(),
                        )
                        .context_menu(move |menu, _window, cx| {
                            let state = prompt_menu_input.read(cx);
                            let has_selection = !state.selected_range().is_empty();
                            let has_text = !state.value().is_empty();
                            let focus = state.focus_handle(cx);
                            menu.action_context(focus)
                                .menu_with_disabled(
                                    i18n::string("workspace.menu.cut"),
                                    Box::new(gpui_component::input::Cut),
                                    !has_selection,
                                )
                                .menu_with_disabled(
                                    i18n::string("workspace.menu.copy"),
                                    Box::new(gpui_component::input::Copy),
                                    !has_selection,
                                )
                                .menu_with_disabled(
                                    i18n::string("workspace.menu.paste"),
                                    Box::new(gpui_component::input::Paste),
                                    cx.read_from_clipboard().is_none(),
                                )
                                .item(PopupMenuItem::separator())
                                .menu_with_disabled(
                                    i18n::string("workspace.menu.select_all"),
                                    Box::new(gpui_component::input::SelectAll),
                                    !has_text,
                                )
                        }),
                )
                .child(
                    h_flex()
                        .w_full()
                        .h(px(28.0))
                        .items_center()
                        .gap_2()
                        .child(
                            div().w(px(112.0)).min_w(px(0.0)).child(
                                md3_select(&provider_select)
                                    .small()
                                    .w_full()
                                    .bg(rgb(roles.surface_container_high)),
                            ),
                        )
                        .child(icon_button_with_tooltip(
                            AppIcon::LaptopMinimal,
                            i18n::string(if app.session_agent.exec_mode.is_pty() {
                                "workspace.panel.agent.tooltips.disable_pty"
                            } else {
                                "workspace.panel.agent.tooltips.enable_pty"
                            }),
                            24.0,
                            8.0,
                            Some(if app.session_agent.exec_mode.is_pty() {
                                roles.secondary_container
                            } else {
                                roles.surface_container_high
                            }),
                            Some(if app.session_agent.exec_mode.is_pty() {
                                roles.on_secondary_container
                            } else {
                                text_muted
                            }),
                            None,
                            move |_window, cx| {
                                let entity = pty_toggle_entity.clone();
                                entity.update(cx, |this, cx| {
                                    this.session_agent.exec_mode =
                                        this.session_agent.exec_mode.toggle();
                                    cx.notify();
                                });
                            },
                        ))
                        .child(
                            div().w(px(80.0)).child(
                                md3_select(&app.workspace_forms.agent.agent_mode_select)
                                    .small()
                                    .w_full(),
                            ),
                        )
                        .child(div().flex_1())
                        .child(render_session_agent_token_usage(
                            &app.session_agent,
                            &app.settings_store,
                            text_muted,
                        ))
                        .child(div().min_w(px(4.0)))
                        .child(
                            div()
                                .id("session-agent-send-action")
                                .child(icon_button_with_tooltip(
                                    if waiting {
                                        AppIcon::Pause
                                    } else {
                                        AppIcon::ChevronUp
                                    },
                                    i18n::string(if waiting {
                                        "workspace.panel.agent.tooltips.stop_response"
                                    } else {
                                        "workspace.panel.agent.tooltips.send_message"
                                    }),
                                    26.0,
                                    8.0,
                                    Some(if waiting {
                                        roles.error_container
                                    } else {
                                        roles.primary
                                    }),
                                    Some(if waiting {
                                        roles.on_error_container
                                    } else {
                                        roles.on_primary
                                    }),
                                    None,
                                    move |window, cx| {
                                        let entity = send_entity.clone();
                                        entity.update(cx, |this, cx| {
                                            if this.session_agent.is_busy() {
                                                this.stop_session_agent_stream(cx);
                                            } else {
                                                this.submit_session_agent_prompt(window, cx);
                                            }
                                        });
                                    },
                                ))
                                .with_animation(
                                    SharedString::from(format!(
                                        "session-agent-send-state-{waiting}"
                                    )),
                                    if waiting {
                                        Animation::new(SESSION_AGENT_SEND_PULSE_DURATION)
                                            .repeat()
                                            .with_easing(gpui::bounce(gpui::ease_in_out))
                                    } else {
                                        short_feedback_animation()
                                    },
                                    move |element, delta| {
                                        if waiting {
                                            element.opacity(0.72 + delta * 0.28)
                                        } else {
                                            element.opacity(0.64 + delta * 0.36)
                                        }
                                    },
                                ),
                        ),
                ),
        )
        .into_any_element()
}
