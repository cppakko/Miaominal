use super::super::super::super::*;
use crate::ui::i18n;
use gpui::InteractiveElement;

pub(in crate::ui::shell::pages::forward) fn draggable_profile_tile(
    profile: &SessionProfile,
    online_sessions: usize,
) -> impl IntoElement {
    let material = settings::current_theme().material;
    let roles = material.roles;
    let extended = material.extended;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let drag_payload = DraggedForwardProfile {
        profile_id: profile.id.clone(),
        name: profile.name.clone(),
        summary: profile.summary(),
    };

    let mut tile = div()
        .min_w(px(220.0))
        .max_w(px(280.0))
        .rounded(px(18.0))
        .bg(rgb(roles.surface_container_high))
        .px_4()
        .py_3()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, _, _| {})
        .child(
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .text_size(settings::scaled_font_size(13.0))
                                .text_color(rgb(roles.on_surface))
                                .child(profile.name.clone()),
                        )
                        .child(badge(
                            if online_sessions > 0 {
                                let count = online_sessions.to_string();
                                if online_sessions == 1 {
                                    i18n::string_args(
                                        "forwarding.tile.online_sessions_one",
                                        &[("count", &count)],
                                    )
                                } else {
                                    i18n::string_args(
                                        "forwarding.tile.online_sessions_other",
                                        &[("count", &count)],
                                    )
                                }
                            } else {
                                i18n::string("forwarding.status.idle")
                            },
                            if online_sessions > 0 {
                                extended.info.color
                            } else {
                                roles.surface_container_low
                            },
                            roles.on_surface,
                        )),
                )
                .child(
                    div()
                        .text_size(settings::scaled_font_size(11.0))
                        .text_color(rgb(roles.on_surface_variant))
                        .child(profile.summary()),
                )
                .child(
                    div()
                        .text_size(settings::scaled_font_size(10.0))
                        .text_color(rgb(text_muted))
                        .child(i18n::string("forwarding.tile.drag_to_target_composer")),
                ),
        );

    tile.interactivity()
        .on_drag(drag_payload, |drag, _, _, cx| cx.new(|_| drag.clone()));
    tile
}