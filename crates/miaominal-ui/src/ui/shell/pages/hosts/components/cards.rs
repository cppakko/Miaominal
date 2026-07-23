use super::super::super::super::*;

#[derive(Clone, Debug)]
pub(in crate::ui::shell::pages::hosts) struct HostCardBadgeChip {
    pub label: SharedString,
    pub tooltip: Option<SharedString>,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell::pages::hosts) struct HostCardGroupBadge {
    pub badge: HostCardBadgeChip,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell::pages::hosts) struct HostCardTags {
    pub visible: Vec<HostCardBadgeChip>,
    pub overflow: Option<HostCardBadgeChip>,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell::pages::hosts) struct HostCardMetadata {
    pub group: Option<HostCardGroupBadge>,
    pub tags: HostCardTags,
}

fn host_card_badge_tooltip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView {
    move |window, cx| gpui_component::tooltip::Tooltip::new(text.clone()).build(window, cx)
}

fn host_card_badge(
    id: SharedString,
    label: SharedString,
    tooltip: Option<SharedString>,
    background: u32,
    foreground: u32,
) -> gpui::AnyElement {
    div()
        .id(id)
        .flex_shrink_0()
        .when_some(tooltip, |this, tooltip| {
            this.tooltip(host_card_badge_tooltip(tooltip))
        })
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
    let roles = miaominal_settings::current_theme().material.roles;
    let icon = icon.into();
    let count = count.into();
    let title = title.into();
    let item_id = SharedString::from(format!("host-group-card-{}", title.as_ref()));

    card_surface(
        if is_selected {
            palette.accent_container
        } else {
            roles.surface_container
        },
        18.0,
    )
    .id(item_id)
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
                            .text_size(miaominal_settings::FontSize::Input.scaled())
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
                        .text_size(miaominal_settings::FontSize::Heading.scaled())
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
    metadata: HostCardMetadata,
    badge_id_prefix: impl Into<SharedString>,
    action_icon: AppIcon,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    on_action_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let title = title.into();
    let badge_id_prefix = badge_id_prefix.into();
    let HostCardMetadata { group, tags } = metadata;
    let has_metadata = group.is_some() || !tags.visible.is_empty() || tags.overflow.is_some();
    let group_badge = group.map(|group| {
        host_card_badge(
            SharedString::from(format!("{badge_id_prefix}-group")),
            group.badge.label,
            group.badge.tooltip,
            roles.surface_container_highest,
            roles.on_surface_variant,
        )
    });
    let tag_badges: Vec<_> = tags
        .visible
        .into_iter()
        .enumerate()
        .map(|(index, tag)| {
            host_card_badge(
                SharedString::from(format!("{badge_id_prefix}-tag-{index}")),
                tag.label,
                tag.tooltip,
                roles.surface_container_low,
                roles.on_surface_variant,
            )
        })
        .collect();
    let overflow_badge = tags.overflow.map(|overflow| {
        host_card_badge(
            SharedString::from(format!("{badge_id_prefix}-overflow")),
            overflow.label,
            overflow.tooltip,
            roles.surface_container_highest,
            roles.on_surface_variant,
        )
    });

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
                                            .text_size(miaominal_settings::FontSize::Body.scaled())
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
                                                .text_size(miaominal_settings::scaled_font_size(
                                                    14.0,
                                                ))
                                                .text_color(rgb(roles.on_surface))
                                                .child(title),
                                        )
                                        .when(has_metadata, move |this| {
                                            this.child(
                                                h_flex()
                                                    .gap_1()
                                                    .min_w(px(0.0))
                                                    .overflow_hidden()
                                                    .children(group_badge)
                                                    .children(tag_badges)
                                                    .children(overflow_badge),
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
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );

    list_item_card(
        icon_tile(
            div()
                .text_size(miaominal_settings::FontSize::Body.scaled())
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
                    .text_size(miaominal_settings::FontSize::Subheading.scaled())
                    .text_color(rgb(roles.on_surface))
                    .child(title),
            )
            .children(subtitle.into_iter().map(|subtitle| {
                div()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
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
