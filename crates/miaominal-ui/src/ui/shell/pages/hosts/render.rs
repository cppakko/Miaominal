use super::super::super::*;
use super::super::empty_state::{shell_empty_page, shell_empty_state};
use super::components::{
    HostCardBadgeChip, HostCardGroupBadge, HostCardMetadata, HostCardTags, group_card,
    host_card_with_action, host_list_row,
};
use crate::ui::i18n;
use miaominal_core::profile::SessionProfile;
use std::{collections::BTreeMap, rc::Rc};

const HOST_CARD_TAG_ROW_UNIT_BUDGET: usize = 30;
const HOST_CARD_TAG_BADGE_UNIT_OVERHEAD: usize = 4;
const HOST_CARD_TAG_GAP_UNITS: usize = 1;
const HOST_CARD_TAG_MIN_LABEL_UNITS: usize = 4;

type HostPageAction = Rc<dyn Fn(usize, &mut Window, &mut App)>;

#[derive(Clone)]
struct HostPageActions {
    connect: HostPageAction,
    edit: HostPageAction,
    open_sftp: HostPageAction,
}

impl HostPageActions {
    fn new(
        connect: impl Fn(usize, &mut Window, &mut App) + 'static,
        edit: impl Fn(usize, &mut Window, &mut App) + 'static,
        open_sftp: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            connect: Rc::new(connect),
            edit: Rc::new(edit),
            open_sftp: Rc::new(open_sftp),
        }
    }

    fn connect(&self, index: usize, window: &mut Window, cx: &mut App) {
        (self.connect)(index, window, cx);
    }

    fn edit(&self, index: usize, window: &mut Window, cx: &mut App) {
        (self.edit)(index, window, cx);
    }

    fn open_sftp(&self, index: usize, window: &mut Window, cx: &mut App) {
        (self.open_sftp)(index, window, cx);
    }
}

fn build_host_context_menu(
    menu: PopupMenu,
    controller: Entity<SessionController>,
    actions: HostPageActions,
    index: usize,
    is_favorite: bool,
) -> PopupMenu {
    let favorite_controller = controller.clone();
    let duplicate_controller = controller.clone();
    let delete_controller = controller;

    let fav_label = if is_favorite {
        i18n::string("hosts.menu.remove_from_favorites")
    } else {
        i18n::string("hosts.menu.add_to_favorites")
    };

    menu.item(PopupMenuItem::new(fav_label).on_click(move |_, _, cx| {
        favorite_controller.update(cx, |controller, cx| {
            controller.toggle_profile_favorite(index, cx);
        });
    }))
    .item(
        PopupMenuItem::new(i18n::string("hosts.menu.open_sftp")).on_click(move |_, window, cx| {
            actions.open_sftp(index, window, cx);
        }),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("hosts.menu.duplicate_profile")).on_click(
            move |_, _, cx| {
                duplicate_controller.update(cx, |controller, cx| {
                    controller.duplicate_profile_at_index(index, cx);
                });
            },
        ),
    )
    .item(
        PopupMenuItem::new(i18n::string("hosts.menu.delete_profile")).on_click(move |_, _, cx| {
            delete_controller.update(cx, |controller, cx| {
                controller.request_profile_delete_at_index(index, cx);
            });
        }),
    )
}

fn group_icon(group: &str) -> String {
    let mut icon = String::new();
    for token in group
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
    {
        if let Some(ch) = token.chars().next() {
            icon.push(ch.to_ascii_uppercase());
        }
        if icon.len() >= 3 {
            break;
        }
    }

    if icon.is_empty() {
        for ch in group.chars().filter(|ch| ch.is_alphanumeric()) {
            icon.push(ch.to_ascii_uppercase());
            if icon.len() >= 3 {
                break;
            }
        }
    }

    if icon.is_empty() {
        i18n::string("hosts.groups.fallback_icon")
    } else {
        icon
    }
}

fn group_accent(name: &str) -> GroupAccentPalette {
    let material = miaominal_settings::current_theme().material;

    group_accent_palette(name, &material)
}

fn profile_subtitle(profile: &SessionProfile) -> Option<String> {
    let group = profile.group.trim();
    (!group.is_empty()).then(|| group.to_string())
}

fn host_card_tag_display_units(label: &str) -> usize {
    label
        .chars()
        .map(|character| if character.is_ascii() { 1 } else { 2 })
        .sum()
}

fn host_card_tag_badge_units(label: &str) -> usize {
    HOST_CARD_TAG_BADGE_UNIT_OVERHEAD + host_card_tag_display_units(label)
}

fn truncate_host_card_badge_to_units(label: &str, max_label_units: usize) -> String {
    if host_card_tag_display_units(label) <= max_label_units {
        return label.to_string();
    }

    if max_label_units <= 3 {
        return "...".to_string();
    }

    let mut visible = String::new();
    let mut used_units = 0;
    let visible_units_budget = max_label_units.saturating_sub(3);

    for character in label.chars() {
        let character_units = if character.is_ascii() { 1 } else { 2 };
        if used_units + character_units > visible_units_budget {
            break;
        }

        visible.push(character);
        used_units += character_units;
    }

    if visible.is_empty() {
        "...".to_string()
    } else {
        format!("{visible}...")
    }
}

fn prepare_host_card_tags(raw_tags: &[String], row_unit_budget: usize) -> HostCardTags {
    let mut unique_tags = Vec::new();

    for raw_tag in raw_tags {
        let tag = raw_tag.trim();
        if tag.is_empty()
            || unique_tags
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(tag))
        {
            continue;
        }

        unique_tags.push(tag.to_string());
    }

    let mut used_row_units = 0;
    let mut visible = Vec::new();
    let mut visible_source_count = 0;

    for (index, tag) in unique_tags.iter().enumerate() {
        let gap_before_tag_units = if visible.is_empty() {
            0
        } else {
            HOST_CARD_TAG_GAP_UNITS
        };
        let available_badge_units =
            row_unit_budget.saturating_sub(used_row_units + gap_before_tag_units);
        let full_badge_units = host_card_tag_badge_units(tag);

        if full_badge_units <= available_badge_units {
            visible.push(HostCardBadgeChip {
                label: SharedString::from(tag.clone()),
                tooltip: None,
            });
            used_row_units += gap_before_tag_units + full_badge_units;
            visible_source_count = index + 1;
            continue;
        }

        let available_label_units =
            available_badge_units.saturating_sub(HOST_CARD_TAG_BADGE_UNIT_OVERHEAD);
        if available_label_units < HOST_CARD_TAG_MIN_LABEL_UNITS {
            break;
        }

        let label = truncate_host_card_badge_to_units(tag, available_label_units);
        let truncated_badge_units = host_card_tag_badge_units(&label);
        visible.push(HostCardBadgeChip {
            tooltip: (label != *tag).then(|| SharedString::from(tag.clone())),
            label: SharedString::from(label),
        });
        used_row_units += gap_before_tag_units + truncated_badge_units;
        visible_source_count = index + 1;
        break;
    }

    let overflow_count = unique_tags.len().saturating_sub(visible_source_count);
    let overflow = if overflow_count > 0 {
        let label = format!("+{overflow_count}");
        let gap_units = if visible.is_empty() {
            0
        } else {
            HOST_CARD_TAG_GAP_UNITS
        };
        if used_row_units + gap_units + host_card_tag_badge_units(&label) <= row_unit_budget {
            Some(HostCardBadgeChip {
                label: SharedString::from(label),
                tooltip: Some(SharedString::from(
                    unique_tags[visible_source_count..]
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                )),
            })
        } else {
            None
        }
    } else {
        None
    };

    if overflow_count > 0
        && overflow.is_none()
        && let Some(last_visible) = visible.last_mut()
    {
        let tooltip_start = visible_source_count.saturating_sub(1);
        last_visible.tooltip = Some(SharedString::from(
            unique_tags[tooltip_start..]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }

    HostCardTags { visible, overflow }
}

fn prepare_host_card_metadata(profile: &SessionProfile) -> HostCardMetadata {
    let group_name = profile.group.trim();
    let (group, tags_row_unit_budget) = if group_name.is_empty() {
        (None, HOST_CARD_TAG_ROW_UNIT_BUDGET)
    } else {
        let available_label_units =
            HOST_CARD_TAG_ROW_UNIT_BUDGET.saturating_sub(HOST_CARD_TAG_BADGE_UNIT_OVERHEAD);
        let label = truncate_host_card_badge_to_units(group_name, available_label_units);
        let group_units = host_card_tag_badge_units(&label);
        let remaining_units = HOST_CARD_TAG_ROW_UNIT_BUDGET.saturating_sub(group_units);
        let tags_row_unit_budget = remaining_units.saturating_sub(HOST_CARD_TAG_GAP_UNITS);

        (
            Some(HostCardGroupBadge {
                badge: HostCardBadgeChip {
                    tooltip: (label != group_name)
                        .then(|| SharedString::from(group_name.to_string())),
                    label: SharedString::from(label),
                },
            }),
            tags_row_unit_budget,
        )
    };

    HostCardMetadata {
        group,
        tags: prepare_host_card_tags(&profile.tags, tags_row_unit_budget),
    }
}

fn render_host_profile_item(
    controller: Entity<SessionController>,
    actions: HostPageActions,
    index: usize,
    profile: &SessionProfile,
    is_list: bool,
    id_prefix: &'static str,
) -> gpui::AnyElement {
    let display_title = truncate_with_ellipsis(&profile.name, if is_list { 40 } else { 18 });
    let is_favorite = profile.is_favorite;
    let item_id = SharedString::from(format!(
        "{id_prefix}-{}-{}",
        if is_list { "row" } else { "card" },
        profile.id
    ));
    let badge_id_prefix = SharedString::from(format!("{id_prefix}-card-metadata-{}", profile.id));
    let menu_controller = controller;
    let menu_actions = actions.clone();
    let connect_actions = actions.clone();
    let edit_actions = actions;

    if is_list {
        let subtitle = profile_subtitle(profile).map(SharedString::from);
        div()
            .id(item_id)
            .w_full()
            .context_menu(move |menu, _window, _cx| {
                build_host_context_menu(
                    menu,
                    menu_controller.clone(),
                    menu_actions.clone(),
                    index,
                    is_favorite,
                )
            })
            .child(host_list_row(
                SharedString::from(display_title),
                subtitle,
                None,
                0,
                Some(AppIcon::Edit),
                move |window, cx| connect_actions.connect(index, window, cx),
                move |window, cx| edit_actions.edit(index, window, cx),
            ))
            .into_any_element()
    } else {
        let metadata = prepare_host_card_metadata(profile);
        div()
            .id(item_id)
            .w(px(HOST_CARD_WIDTH))
            .context_menu(move |menu, _window, _cx| {
                build_host_context_menu(
                    menu,
                    menu_controller.clone(),
                    menu_actions.clone(),
                    index,
                    is_favorite,
                )
            })
            .child(host_card_with_action(
                display_title,
                metadata,
                badge_id_prefix,
                AppIcon::Edit,
                move |window, cx| connect_actions.connect(index, window, cx),
                move |window, cx| edit_actions.edit(index, window, cx),
            ))
            .into_any_element()
    }
}

impl SessionController {
    pub(in crate::ui::shell) fn render_hosts_page(
        &self,
        controller: Entity<Self>,
        on_connect: impl Fn(usize, &mut Window, &mut App) + 'static,
        on_edit: impl Fn(usize, &mut Window, &mut App) + 'static,
        on_open_sftp: impl Fn(usize, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> gpui::AnyElement {
        let actions = HostPageActions::new(on_connect, on_edit, on_open_sftp);
        let hosts_filter_input = self.panel_forms().hosts.filter_input;
        let catalog_view = self.catalog_view();
        let profiles = self.profiles();
        let filter_text = hosts_filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let search_matched_sessions: Vec<_> = profiles
            .iter()
            .enumerate()
            .filter(|(_, profile)| {
                if filter_text.is_empty() {
                    return true;
                }

                let haystack = format!(
                    "{} {} {} {}",
                    profile.name.to_ascii_lowercase(),
                    profile.group.to_ascii_lowercase(),
                    profile.host.to_ascii_lowercase(),
                    profile.username.to_ascii_lowercase()
                );
                haystack.contains(&filter_text)
            })
            .collect();
        let selected_group_filter = catalog_view.hosts_group_filter.as_deref();
        let visible_sessions: Vec<_> = search_matched_sessions
            .iter()
            .copied()
            .filter(|(_, profile)| {
                selected_group_filter.is_none_or(|group| profile.group.trim() == group)
            })
            .collect();
        let favorite_sessions: Vec<_> = visible_sessions
            .iter()
            .copied()
            .filter(|(_, profile)| profile.is_favorite)
            .collect();
        let recent_connections_count =
            miaominal_settings::current_settings().recent_connections_count as usize;
        let recent_sessions: Vec<_> = if recent_connections_count == 0 {
            Vec::new()
        } else {
            let mut with_time: Vec<_> = profiles
                .iter()
                .enumerate()
                .filter_map(|(index, profile)| {
                    profile.last_connected_at.map(|ts| (ts, index, profile))
                })
                .collect();
            with_time.sort_by_key(|entry| std::cmp::Reverse(entry.0));
            with_time
                .into_iter()
                .take(recent_connections_count)
                .map(|(_, index, profile)| (index, profile))
                .collect()
        };

        if profiles.is_empty() {
            return shell_empty_page(AppIcon::Computer, i18n::string("hosts.empty.no_profiles"))
                .into_any_element();
        }

        let mut grouped_sessions: BTreeMap<String, usize> = BTreeMap::new();
        for (_, profile) in &search_matched_sessions {
            let group = profile.group.trim();
            if group.is_empty() {
                continue;
            }

            let entry = grouped_sessions.entry(group.to_string()).or_insert(0);
            *entry += 1;
        }
        let group_summaries: Vec<_> = grouped_sessions
            .into_iter()
            .map(|(group_name, host_count)| {
                let host_count_text = host_count.to_string();
                let count = if host_count == 1 {
                    i18n::string_args(
                        "hosts.groups.host_count_one",
                        &[("count", &host_count_text)],
                    )
                } else {
                    i18n::string_args(
                        "hosts.groups.host_count_other",
                        &[("count", &host_count_text)],
                    )
                };
                (
                    group_icon(&group_name),
                    count,
                    group_name.clone(),
                    group_accent(&group_name),
                )
            })
            .collect();
        let group_filter_controller = controller.clone();
        let is_list = catalog_view.hosts_view_mode == ProfileViewMode::List;

        let content = v_flex()
            .w_full()
            .gap_7()
            .px_5()
            .pb_8()
            .when(!group_summaries.is_empty(), move |this| {
                this.child(
                    v_flex()
                        .gap_4()
                        .child(page_section_title(i18n::string("hosts.page.groups")))
                        .child(
                            div().flex().flex_wrap().gap_4().children(
                                group_summaries
                                    .into_iter()
                                    .map(|(icon, count, title, accent)| {
                                        let group_name = title.clone();
                                        let is_selected =
                                            selected_group_filter == Some(group_name.as_str());
                                        group_card(icon, count, title, accent, is_selected, {
                                            let controller = group_filter_controller.clone();
                                            move |_, cx| {
                                                let group_name = group_name.clone();
                                                controller.update(cx, |controller, cx| {
                                                    controller.toggle_hosts_group_filter(
                                                        group_name.clone(),
                                                        cx,
                                                    );
                                                });
                                            }
                                        })
                                    }),
                            ),
                        ),
                )
            })
            .when(
                !recent_sessions.is_empty() && selected_group_filter.is_none(),
                {
                    let mut recent_connections = if is_list {
                        v_flex().w_full().gap_2()
                    } else {
                        div().flex().flex_wrap().gap_4()
                    };
                    for (index, profile) in recent_sessions {
                        recent_connections = recent_connections.child(render_host_profile_item(
                            controller.clone(),
                            actions.clone(),
                            index,
                            profile,
                            is_list,
                            "recent",
                        ));
                    }
                    move |this| {
                        this.child(
                            v_flex()
                                .gap_4()
                                .child(page_section_title(i18n::string(
                                    "hosts.page.recent_connections",
                                )))
                                .child(recent_connections),
                        )
                    }
                },
            )
            .when(!favorite_sessions.is_empty(), {
                let mut fav_connections = if is_list {
                    v_flex().w_full().gap_2()
                } else {
                    div().flex().flex_wrap().gap_4()
                };
                for (index, profile) in favorite_sessions {
                    fav_connections = fav_connections.child(render_host_profile_item(
                        controller.clone(),
                        actions.clone(),
                        index,
                        profile,
                        is_list,
                        "fav",
                    ));
                }
                move |this| {
                    this.child(
                        v_flex()
                            .gap_4()
                            .child(page_section_title(i18n::string("hosts.page.favorites")))
                            .child(fav_connections),
                    )
                }
            })
            .child({
                let mut connections = if is_list {
                    v_flex().w_full().gap_2()
                } else {
                    div().flex().flex_wrap().gap_4()
                };

                if visible_sessions.is_empty() {
                    connections = connections.child(shell_empty_state(
                        AppIcon::Computer,
                        i18n::string("hosts.empty.no_filter_matches"),
                    ));
                } else {
                    for (index, profile) in visible_sessions {
                        connections = connections.child(render_host_profile_item(
                            controller.clone(),
                            actions.clone(),
                            index,
                            profile,
                            is_list,
                            "host",
                        ));
                    }
                }

                v_flex()
                    .gap_4()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_3()
                            .child(page_section_title(i18n::string(
                                "hosts.page.active_connections",
                            ))),
                    )
                    .child(connections)
            });

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                v_flex()
                    .w_full()
                    .px_5()
                    .gap_7()
                    .child(
                        h_flex().w_full().justify_center().child(
                            h_flex()
                                .w_full()
                                .max_w(px(576.0))
                                .child(search_filter_input(
                                    &hosts_filter_input,
                                    SearchInputStyle::Pill,
                                    None,
                                )),
                        ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .justify_end()
                            .gap_2()
                            .child(page_view_mode_toolbar_item(AppIcon::Grid, !is_list, {
                                let controller = controller.clone();
                                move |_, cx| {
                                    controller.update(cx, |controller, cx| {
                                        controller.set_hosts_view_mode(ProfileViewMode::Grid, cx);
                                    });
                                }
                            }))
                            .child(page_view_mode_toolbar_item(AppIcon::List, is_list, {
                                let controller = controller.clone();
                                move |_, cx| {
                                    controller.update(cx, |controller, cx| {
                                        controller.set_hosts_view_mode(ProfileViewMode::List, cx);
                                    });
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .pr_5()
                    .child(div().size_full().overflow_y_scrollbar().child(content)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_subtitle_uses_group_when_present() {
        let mut profile = SessionProfile::blank("profile-1", 1);
        profile.group = "Production".into();
        profile.host = "10.0.0.5".into();

        assert_eq!(profile_subtitle(&profile), Some("Production".to_string()));
    }

    #[test]
    fn profile_subtitle_is_none_without_group() {
        let mut profile = SessionProfile::blank("profile-1", 1);
        profile.group = "   ".into();
        profile.host = "10.0.0.5".into();

        assert_eq!(profile_subtitle(&profile), None);
    }

    #[test]
    fn profile_subtitle_does_not_fall_back_to_host() {
        let mut profile = SessionProfile::blank("profile-1", 1);
        profile.host = "192.168.1.10".into();

        assert_eq!(profile_subtitle(&profile), None);
    }

    #[test]
    fn prepare_host_card_tags_deduplicates_and_keeps_visible_tags() {
        let tags = vec![
            " jump ".to_string(),
            "JUMP".to_string(),
            "ops".to_string(),
            "db".to_string(),
            "production".to_string(),
        ];

        let summary = prepare_host_card_tags(&tags, HOST_CARD_TAG_ROW_UNIT_BUDGET);

        assert_eq!(summary.visible.len(), 3);
        assert_eq!(summary.visible[0].label.as_ref(), "jump");
        assert_eq!(summary.visible[0].tooltip, None);
        assert_eq!(summary.visible[1].label.as_ref(), "ops");
        assert_eq!(summary.visible[1].tooltip, None);
        assert_eq!(summary.visible[2].label.as_ref(), "db");
        assert_eq!(summary.visible[2].tooltip, None);
        assert_eq!(summary.overflow.as_ref().unwrap().label.as_ref(), "+1");
        assert_eq!(
            summary.overflow.as_ref().unwrap().tooltip.as_deref(),
            Some("production")
        );
    }

    #[test]
    fn prepare_host_card_tags_does_not_remove_visible_tags_for_overflow() {
        let tags = vec![
            "jump".to_string(),
            "private-link".to_string(),
            "production".to_string(),
        ];

        let summary = prepare_host_card_tags(&tags, HOST_CARD_TAG_ROW_UNIT_BUDGET);

        assert_eq!(summary.visible.len(), 2);
        assert_eq!(summary.visible[0].label.as_ref(), "jump");
        assert_eq!(summary.visible[1].label.as_ref(), "private-link");
        assert_eq!(
            summary.visible[1].tooltip.as_deref(),
            Some("private-link, production")
        );
        assert!(summary.overflow.is_none());
    }

    #[test]
    fn prepare_host_card_tags_adds_tooltip_for_truncated_visible_tag() {
        let tags = vec!["abcdefghijklmnopqrstuvwxyzabcd".to_string()];

        let summary = prepare_host_card_tags(&tags, HOST_CARD_TAG_ROW_UNIT_BUDGET);

        assert_eq!(summary.visible.len(), 1);
        assert_eq!(
            summary.visible[0].label.as_ref(),
            "abcdefghijklmnopqrstuvw..."
        );
        assert_eq!(
            summary.visible[0].tooltip.as_deref(),
            Some("abcdefghijklmnopqrstuvwxyzabcd")
        );
        assert!(summary.overflow.is_none());
    }

    #[test]
    fn prepare_host_card_metadata_places_group_before_tag_with_shared_budget() {
        let mut profile = SessionProfile::blank("profile-1", 1);
        profile.group = "HK,HKG,BGP,RP-A".into();
        profile.tags = vec!["cccc".into()];

        let metadata = prepare_host_card_metadata(&profile);
        let group = metadata.group.unwrap();

        assert_eq!(group.badge.label.as_ref(), "HK,HKG,BGP,RP-A");
        assert_eq!(group.badge.tooltip, None);
        assert_eq!(metadata.tags.visible.len(), 1);
        assert_eq!(metadata.tags.visible[0].label.as_ref(), "cccc");
        assert_eq!(metadata.tags.visible[0].tooltip, None);
        assert!(metadata.tags.overflow.is_none());
    }

    #[test]
    fn prepare_host_card_metadata_fits_short_second_tag_after_group() {
        let mut profile = SessionProfile::blank("profile-1", 1);
        profile.group = "FinalShell".into();
        profile.tags = vec!["cccc".into(), "a".into()];

        let metadata = prepare_host_card_metadata(&profile);

        assert_eq!(metadata.tags.visible.len(), 2);
        assert_eq!(metadata.tags.visible[0].label.as_ref(), "cccc");
        assert_eq!(metadata.tags.visible[1].label.as_ref(), "a");
        assert_eq!(metadata.tags.visible[0].tooltip, None);
        assert_eq!(metadata.tags.visible[1].tooltip, None);
        assert!(metadata.tags.overflow.is_none());
    }

    #[test]
    fn prepare_host_card_metadata_keeps_first_tag_when_more_tags_do_not_fit() {
        let mut profile = SessionProfile::blank("profile-1", 1);
        profile.group = "HK,HKG,BGP,RP-A".into();
        profile.tags = vec!["cccc".into(), "dddd".into()];

        let metadata = prepare_host_card_metadata(&profile);

        assert_eq!(metadata.tags.visible.len(), 1);
        assert_eq!(metadata.tags.visible[0].label.as_ref(), "cccc");
        assert_eq!(
            metadata.tags.visible[0].tooltip.as_deref(),
            Some("cccc, dddd")
        );
        assert!(metadata.tags.overflow.is_none());
    }

    #[test]
    fn prepare_host_card_metadata_prioritizes_long_group_over_tags() {
        let mut profile = SessionProfile::blank("profile-1", 1);
        profile.group = "abcdefghijklmnopqrstuvwxyzabcd".into();
        profile.tags = vec!["ops".into(), "db".into()];

        let metadata = prepare_host_card_metadata(&profile);
        let group = metadata.group.unwrap();

        assert_eq!(group.badge.label.as_ref(), "abcdefghijklmnopqrstuvw...");
        assert_eq!(
            group.badge.tooltip.as_deref(),
            Some("abcdefghijklmnopqrstuvwxyzabcd")
        );
        assert!(metadata.tags.visible.is_empty());
        assert!(metadata.tags.overflow.is_none());
    }

    #[test]
    fn prepare_host_card_metadata_uses_full_row_for_tags_without_group() {
        let mut profile = SessionProfile::blank("profile-1", 1);
        profile.tags = vec!["jump".into(), "ops".into()];

        let metadata = prepare_host_card_metadata(&profile);

        assert!(metadata.group.is_none());
        assert_eq!(metadata.tags.visible.len(), 2);
        assert_eq!(metadata.tags.visible[0].label.as_ref(), "jump");
        assert_eq!(metadata.tags.visible[1].label.as_ref(), "ops");
        assert!(metadata.tags.overflow.is_none());
    }

    #[test]
    fn prepare_host_card_metadata_handles_group_only_and_empty_metadata() {
        let mut group_only = SessionProfile::blank("profile-1", 1);
        group_only.group = "Production".into();

        let group_only_metadata = prepare_host_card_metadata(&group_only);
        assert_eq!(
            group_only_metadata.group.unwrap().badge.label.as_ref(),
            "Production"
        );
        assert!(group_only_metadata.tags.visible.is_empty());
        assert!(group_only_metadata.tags.overflow.is_none());

        let empty = SessionProfile::blank("profile-2", 2);
        let empty_metadata = prepare_host_card_metadata(&empty);
        assert!(empty_metadata.group.is_none());
        assert!(empty_metadata.tags.visible.is_empty());
        assert!(empty_metadata.tags.overflow.is_none());
    }

    #[test]
    fn prepare_host_card_tags_shows_only_overflow_when_tags_do_not_fit() {
        let tags = vec!["production".into(), "database".into()];

        let summary = prepare_host_card_tags(&tags, 6);

        assert!(summary.visible.is_empty());
        assert_eq!(summary.overflow.as_ref().unwrap().label.as_ref(), "+2");
        assert_eq!(
            summary.overflow.as_ref().unwrap().tooltip.as_deref(),
            Some("production, database")
        );
    }
}
