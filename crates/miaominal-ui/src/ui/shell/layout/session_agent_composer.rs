use super::super::*;
use super::session_agent_utils::*;
use crate::ui::i18n;

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
                                    "Cut",
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
                                    "Select All",
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
                        .child(icon_button(
                            AppIcon::Plus,
                            24.0,
                            8.0,
                            Some(roles.surface_container_high),
                            Some(text_muted),
                            None,
                            |_window, _cx| {},
                        ))
                        .child(div().h(px(16.0)).w(px(1.0)).bg(rgb(roles.outline_variant)))
                        .child(
                            div().w(px(112.0)).min_w(px(0.0)).child(
                                md3_select(&provider_select)
                                    .small()
                                    .w_full()
                                    .bg(rgb(roles.surface_container_high)),
                            ),
                        )
                        .child(icon_button(
                            AppIcon::Sliders,
                            24.0,
                            8.0,
                            Some(roles.surface_container_high),
                            Some(text_muted),
                            None,
                            |_window, _cx| {},
                        ))
                        .child(icon_button(
                            AppIcon::LaptopMinimal,
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
                        .child(div().flex_1())
                        .child(render_session_agent_token_usage(
                            &app.session_agent,
                            &app.settings_store,
                            text_muted,
                        ))
                        .child(div().min_w(px(4.0)))
                        .child(icon_button(
                            if waiting {
                                AppIcon::Pause
                            } else {
                                AppIcon::ChevronUp
                            },
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
                        )),
                ),
        )
        .into_any_element()
}
