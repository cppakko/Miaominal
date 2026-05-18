use super::super::super::super::*;

#[derive(Clone, Debug)]
pub(in crate::ui::shell::pages::hosts) struct HostCardTagChip {
    pub label: SharedString,
    pub tooltip: Option<SharedString>,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell::pages::hosts) struct HostCardTags {
    pub visible: Vec<HostCardTagChip>,
    pub overflow_count: usize,
    pub overflow_tooltip: Option<SharedString>,
}

fn host_card_tag_badge(
    label: SharedString,
    _tooltip: Option<SharedString>,
    background: u32,
    foreground: u32,
) -> gpui::AnyElement {
    div()
        .flex_shrink_0()
        .child(badge(label, background, foreground))
        .into_any_element()
}

pub(in crate::ui::shell::pages::hosts) fn group_card(
    icon: impl Into<SharedString>,
    count: impl Into<SharedString>,
    title: impl Into<SharedString>,
    palette: GroupAccentPalette,
    is_selected: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let roles = settings::current_theme().material.roles;
    let icon = icon.into();
    let count = count.into();
    let title = title.into();

    card_surface(
        if is_selected {
            palette.accent_container
        } else {
            roles.surface_container
        },
        18.0,
    )
    .w(px(GROUP_CARD_WIDTH))
    .h(px(GROUP_CARD_HEIGHT))
    .cursor_pointer()
    .p_4()
    .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, cx| {
        on_click(window, cx);
    })
    .child(
        v_flex()
            .size_full()
            .justify_between()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .size(px(38.0))
                            .rounded(px(12.0))
                            .bg(rgb(if is_selected {
                                palette.accent
                            } else {
                                roles.surface_container_low
                            }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(settings::scaled_font_size(12.0))
                            .text_color(rgb(if is_selected {
                                palette.on_accent
                            } else {
                                palette.accent
                            }))
                            .child(icon),
                    )
                    .child(badge(
                        count,
                        roles.surface_container_highest,
                        if is_selected {
                            palette.on_accent_container
                        } else {
                            roles.on_surface_variant
                        },
                    )),
            )
            .child(
                v_flex().gap_1().child(
                    div()
                        .text_size(settings::scaled_font_size(14.0))
                        .text_color(rgb(if is_selected {
                            palette.on_accent_container
                        } else {
                            roles.on_surface
                        }))
                        .child(title),
                ),
            ),
    )
}

pub(in crate::ui::shell::pages::hosts) fn host_card_with_action(
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    _status_label: Option<SharedString>,
    _status_color: u32,
    tags: HostCardTags,
    action_icon: AppIcon,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    on_action_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let material = settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let title = title.into();
    let has_tags = !tags.visible.is_empty() || tags.overflow_count > 0;
    let overflow_badge =
        (tags.overflow_count > 0).then(|| SharedString::from(format!("+{}", tags.overflow_count)));

    card_surface(roles.surface_container, 18.0)
        .w(px(HOST_CARD_WIDTH))
        .h(px(HOST_CARD_HEIGHT))
        .p_4()
        .child(
            h_flex()
                .size_full()
                .gap_1()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .h_full()
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, cx| {
                            on_click(window, cx);
                        })
                        .child(
                            h_flex()
                                .w_full()
                                .h_full()
                                .items_center()
                                .gap_3()
                                .child(
                                    icon_tile(
                                        div()
                                            .text_size(settings::scaled_font_size(11.0))
                                            .child(">_"),
                                        34.0,
                                        10.0,
                                        IconTileTone::Muted,
                                    )
                                    .flex_shrink_0(),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .gap_1()
                                        .child(
                                            div()
                                                .w_full()
                                                .min_w(px(0.0))
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_size(settings::scaled_font_size(14.0))
                                                .text_color(rgb(roles.on_surface))
                                                .child(title),
                                        )
                                        .children(subtitle.into_iter().map(|subtitle| {
                                            div()
                                                .w_full()
                                                .min_w(px(0.0))
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_size(settings::scaled_font_size(11.0))
                                                .line_height(settings::scaled_line_height(16.0))
                                                .text_color(rgb(text_muted))
                                                .child(subtitle)
                                        }))
                                        .when(has_tags, move |this| {
                                            this.child(
                                                h_flex()
                                                    .gap_2()
                                                    .min_w(px(0.0))
                                                    .overflow_hidden()
                                                    .children(tags.visible.into_iter().map(|tag| {
                                                        host_card_tag_badge(
                                                            tag.label,
                                                            tag.tooltip,
                                                            roles.surface_container_low,
                                                            roles.on_surface_variant,
                                                        )
                                                    }))
                                                    .when_some(
                                                        overflow_badge,
                                                        move |this, label| {
                                                            this.child(host_card_tag_badge(
                                                                label,
                                                                tags.overflow_tooltip.clone(),
                                                                roles.surface_container_highest,
                                                                roles.on_surface_variant,
                                                            ))
                                                        },
                                                    ),
                                            )
                                        }),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(30.0))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .child(icon_button(
                            action_icon,
                            30.0,
                            10.0,
                            None,
                            None,
                            Some(roles.outline_variant),
                            move |window, cx| on_action_click(window, cx),
                        )),
                ),
        )
}

pub(in crate::ui::shell::pages::hosts) fn host_list_row(
    title: SharedString,
    subtitle: Option<SharedString>,
    _status_label: Option<SharedString>,
    _status_color: u32,
    action_icon: Option<AppIcon>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    on_action_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let material = settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );

    list_item_card(
        icon_tile(
            div()
                .text_size(settings::scaled_font_size(11.0))
                .child(">_"),
            30.0,
            10.0,
            IconTileTone::Muted,
        )
        .flex_shrink_0()
        .into_any_element(),
        v_flex()
            .flex_1()
            .min_w(px(0.0))
            .gap_1()
            .child(
                div()
                    .text_size(settings::scaled_font_size(13.0))
                    .text_color(rgb(roles.on_surface))
                    .child(title),
            )
            .children(subtitle.into_iter().map(|subtitle| {
                div()
                    .text_size(settings::scaled_font_size(11.0))
                    .text_color(rgb(text_muted))
                    .child(subtitle)
            }))
            .into_any_element(),
        None,
        action_icon.map(|icon| {
            icon_button(icon, 30.0, 10.0, None, None, Some(roles.outline_variant), {
                move |window, cx| on_action_click(window, cx)
            })
            .into_any_element()
        }),
        move |window, cx| on_click(window, cx),
    )
}
