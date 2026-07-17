use super::super::*;
pub(in crate::ui::shell::layout) fn advance_terminal_search_overlay(
    app: &mut AppView,
    window: &mut Window,
    cx: &App,
) -> Option<f32> {
    let controller = app.controllers.session.read(cx);
    controller.sync_terminal_search_target(app.workspace.workspace.active_tab);
    let mut search = controller.terminal_search_mut();

    if let Some(animation) = search.animation {
        let duration_seconds = animation.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            search.visibility = animation.to;
            search.animation = None;
        } else {
            let elapsed = Instant::now().saturating_duration_since(animation.started_at);
            let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
            let eased = progress * progress * (3.0 - 2.0 * progress);
            search.visibility = animation.from + (animation.to - animation.from) * eased;

            if progress >= 1.0 {
                search.visibility = animation.to;
                search.animation = None;
            } else {
                window.request_animation_frame();
            }
        }
    }

    if search.visibility <= f32::EPSILON && !search.open {
        search.visible = false;
        search.total = 0;
        search.current = None;
        search.status = None;
        return None;
    }

    if search.open || search.visibility > f32::EPSILON {
        search.visible = true;
        return Some(search.visibility.clamp(0.0, 1.0));
    }

    search.visible = false;
    None
}

pub(in crate::ui::shell::layout) fn render_terminal_search_overlay(
    app: &AppView,
    visibility: f32,
    cx: &mut Context<AppView>,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let button_background = roles.surface_container_high;
    let (input, total, current, status) = {
        let search = app.controllers.session.read(cx).terminal_search();
        (
            search.input.clone(),
            search.total,
            search.current,
            search.status.clone(),
        )
    };
    let counter = if let Some(message) = status {
        message
    } else if total == 0 {
        "0/0".to_string()
    } else {
        let display_index = current.map(|i| i + 1).unwrap_or(0);
        format!("{display_index}/{total}")
    };

    let prev_controller = app.controllers.session.clone();
    let next_controller = app.controllers.session.clone();
    let close_controller = app.controllers.session.clone();
    let terminal_focus = app.workspace.workspace.active_pane.terminal_focus.clone();

    div()
        .absolute()
        .top(px(12.0))
        .right(px(28.0))
        .occlude()
        .w(px(440.0))
        .opacity(visibility)
        .child(
            search_filter_input(
                &input,
                SearchInputStyle::Compact,
                Some(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .pr_1()
                        .child(
                            div()
                                .min_w(px(48.0))
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(text_muted))
                                .child(counter),
                        )
                        .child(icon_button(
                            AppIcon::ChevronUp,
                            24.0,
                            8.0,
                            Some(button_background),
                            Some(text_muted),
                            None,
                            move |_, cx| {
                                prev_controller.update(cx, |controller, cx| {
                                    controller.terminal_search_prev(cx);
                                });
                            },
                        ))
                        .child(icon_button(
                            AppIcon::ChevronDown,
                            24.0,
                            8.0,
                            Some(button_background),
                            Some(text_muted),
                            None,
                            move |_, cx| {
                                next_controller.update(cx, |controller, cx| {
                                    controller.terminal_search_next(cx);
                                });
                            },
                        ))
                        .child(icon_button(
                            AppIcon::Close,
                            24.0,
                            8.0,
                            Some(button_background),
                            Some(text_muted),
                            None,
                            move |window, cx| {
                                close_controller.update(cx, |controller, cx| {
                                    controller.close_terminal_search(&terminal_focus, window, cx);
                                });
                            },
                        ))
                        .into_any_element(),
                ),
            )
            .bg(rgb(roles.surface_container_highest)),
        )
        .into_any_element()
}
