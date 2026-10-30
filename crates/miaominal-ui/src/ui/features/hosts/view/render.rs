use super::components::host_card;
use crate::ui::assets::AppIcon;
use crate::ui::components::shell_tokens::{ShellColors, ShellMetrics, ShellTypography, space};
use crate::ui::components::{animated_card_collection, view_mode_switcher};
use crate::ui::contracts::navigation::ProfileViewMode;
use crate::ui::features::hosts::HostsController;
use crate::ui::i18n;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    Icon, Selectable, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, SharedString, TestSupportExt as _, Window, div};
use miaominal_core::profile::SessionProfile;
use std::{collections::BTreeMap, rc::Rc};

type HostPageAction = Rc<dyn Fn(String, &mut Window, &mut App)>;

#[derive(Clone)]
struct HostPageActions {
    connect: HostPageAction,
    edit: HostPageAction,
    open_sftp: HostPageAction,
}

impl HostPageActions {
    fn new(
        connect: impl Fn(String, &mut Window, &mut App) + 'static,
        edit: impl Fn(String, &mut Window, &mut App) + 'static,
        open_sftp: impl Fn(String, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            connect: Rc::new(connect),
            edit: Rc::new(edit),
            open_sftp: Rc::new(open_sftp),
        }
    }

    fn connect(&self, index: String, window: &mut Window, cx: &mut App) {
        (self.connect)(index, window, cx);
    }

    fn edit(&self, index: String, window: &mut Window, cx: &mut App) {
        (self.edit)(index, window, cx);
    }

    fn open_sftp(&self, index: String, window: &mut Window, cx: &mut App) {
        (self.open_sftp)(index, window, cx);
    }
}

fn build_host_context_menu(
    menu: PopupMenu,
    controller: Entity<HostsController>,
    actions: HostPageActions,
    profile_id: String,
    is_favorite: bool,
    is_local: bool,
    is_ssh: bool,
) -> PopupMenu {
    let favorite_controller = controller.clone();
    let duplicate_controller = controller.clone();
    let rdp_controller = controller.clone();
    let rdp_id = profile_id.clone();
    let delete_controller = controller;
    let favorite_id = profile_id.clone();
    let duplicate_id = profile_id.clone();
    let sftp_id = profile_id.clone();

    let fav_label = if is_favorite {
        i18n::string("hosts.menu.remove_from_favorites")
    } else {
        i18n::string("hosts.menu.add_to_favorites")
    };

    let connect = actions.clone();
    let edit = actions.clone();
    let connect_id = profile_id.clone();
    let edit_id = profile_id.clone();
    let menu = menu
        .item(
            PopupMenuItem::new(i18n::string("hosts.home.connect"))
                .on_click(move |_, window, cx| connect.connect(connect_id.clone(), window, cx)),
        )
        .item(
            PopupMenuItem::new(i18n::string("hosts.home.edit"))
                .on_click(move |_, window, cx| edit.edit(edit_id.clone(), window, cx)),
        );
    let menu = menu.item(PopupMenuItem::new(fav_label).on_click(move |_, _, cx| {
        favorite_controller.update(cx, |controller, cx| {
            if let Some(index) = controller.profile_index(&favorite_id) {
                controller.toggle_profile_favorite(index, cx);
            }
        });
    }));
    let menu = if is_local {
        menu
    } else {
        menu.item(
            PopupMenuItem::new(i18n::string("hosts.menu.open_sftp")).on_click(
                move |_, window, cx| {
                    actions.open_sftp(sftp_id.clone(), window, cx);
                },
            ),
        )
    };

    let menu = if is_ssh {
        menu.item(
            PopupMenuItem::new(i18n::string("rdp.open_menu")).on_click(move |_, _, cx| {
                rdp_controller.update(cx, |_, cx| {
                    cx.emit(crate::ui::contracts::AppCommand::OpenTab(
                        crate::ui::contracts::TabOpenRequest::Rdp {
                            profile_id: rdp_id.clone(),
                        },
                    ));
                });
            }),
        )
    } else {
        menu
    };

    menu.item(PopupMenuItem::separator())
        .item(
            PopupMenuItem::new(i18n::string("hosts.menu.duplicate_profile")).on_click(
                move |_, _, cx| {
                    duplicate_controller.update(cx, |controller, cx| {
                        if let Some(index) = controller.profile_index(&duplicate_id) {
                            controller.duplicate_profile_at_index(index, cx);
                        }
                    });
                },
            ),
        )
        .item(
            PopupMenuItem::new(i18n::string("hosts.menu.delete_profile")).on_click(
                move |_, _, cx| {
                    delete_controller.update(cx, |controller, cx| {
                        if let Some(index) = controller.profile_index(&profile_id) {
                            controller.request_profile_delete_at_index(index, cx);
                        }
                    });
                },
            ),
        )
}

fn collection_columns(width: f32, quick: bool) -> u16 {
    if quick {
        if width > 920.0 {
            4
        } else if width > 520.0 {
            2
        } else {
            1
        }
    } else {
        crate::ui::components::card_grid_columns(width)
    }
}

fn profile_item(
    controller: Entity<HostsController>,
    actions: HostPageActions,
    profile: &SessionProfile,
    list: bool,
    prefix: &'static str,
    cx: &App,
) -> gpui_kit::AnyElement {
    let id = SharedString::from(format!("{prefix}-{}", profile.id));
    let menu_id = profile.id.clone();
    let more_id = menu_id.clone();
    let connect_id = menu_id.clone();
    let is_favorite = profile.is_favorite;
    let is_local = profile.is_local();
    let is_ssh = profile.kind == miaominal_core::profile::ProfileKind::Ssh;
    let right_controller = controller.clone();
    let right_actions = actions.clone();
    let more_actions = actions.clone();
    let more = Button::new(SharedString::from(format!("{id}:more")))
        .ghost()
        .child(Icon::new(AppIcon::MoreHorizontal).size(space(20.0)))
        .text_color(ShellColors::current().muted)
        .accessibility_label(i18n::string_args(
            "hosts.home.more",
            &[("name", &profile.name)],
        ))
        .size(space(40.0))
        .rounded_full()
        .dropdown_menu(move |menu, _, _| {
            build_host_context_menu(
                menu,
                controller.clone(),
                more_actions.clone(),
                more_id.clone(),
                is_favorite,
                is_local,
                is_ssh,
            )
        });
    div()
        .id(id.clone())
        .min_w_0()
        .context_menu(move |menu, _, _| {
            build_host_context_menu(
                menu,
                right_controller.clone(),
                right_actions.clone(),
                menu_id.clone(),
                is_favorite,
                is_local,
                is_ssh,
            )
        })
        .child(host_card(
            id,
            profile,
            list,
            more.into_any_element(),
            cx,
            move |window, cx| actions.connect(connect_id.clone(), window, cx),
        ))
        .into_any_element()
}

impl HostsController {
    pub(in crate::ui) fn render_hosts_page(
        &self,
        controller: Entity<Self>,
        on_connect: impl Fn(String, &mut Window, &mut App) + 'static,
        on_edit: impl Fn(String, &mut Window, &mut App) + 'static,
        on_open_sftp: impl Fn(String, &mut Window, &mut App) + 'static,
        available_width: f32,
        inset: f32,
        cx: &App,
    ) -> gpui_kit::AnyElement {
        let colors = ShellColors::current();
        let actions = HostPageActions::new(on_connect, on_edit, on_open_sftp);
        let catalog = self.catalog_view();
        let profiles = self.profiles();
        let remote: Vec<_> = profiles
            .iter()
            .filter(|profile| !profile.is_local())
            .collect();
        let selected = catalog.hosts_group_filter.as_deref();
        let visible: Vec<_> = remote
            .iter()
            .copied()
            .filter(|profile| selected.is_none_or(|group| profile.group.trim() == group))
            .collect();
        let favorites: Vec<_> = visible
            .iter()
            .copied()
            .filter(|profile| profile.is_favorite)
            .collect();
        let mut recent: Vec<_> = remote
            .iter()
            .copied()
            .filter(|profile| profile.last_connected_at.is_some())
            .collect();
        recent.sort_by_key(|profile| std::cmp::Reverse(profile.last_connected_at));
        recent.truncate(miaominal_settings::current_settings().recent_connections_count as usize);
        recent.extend(profiles.iter().filter(|profile| profile.is_local()));
        let mut groups = BTreeMap::<String, usize>::new();
        for profile in &remote {
            if !profile.group.trim().is_empty() {
                *groups.entry(profile.group.trim().to_owned()).or_default() += 1;
            }
        }
        let is_list = catalog.hosts_view_mode == ProfileViewMode::List;
        let width =
            (available_width - inset * 2.0 - 8.0).clamp(1.0, ShellMetrics::CONTENT_MAX_WIDTH);
        let render_collection = |items: &[&SessionProfile], quick: bool, prefix| {
            let children = items.iter().map(|profile| {
                (
                    SharedString::from(format!("{prefix}-{}-layout", profile.id)),
                    profile_item(
                        controller.clone(),
                        actions.clone(),
                        profile,
                        is_list && !quick,
                        prefix,
                        cx,
                    ),
                )
            });
            if !quick {
                return animated_card_collection(
                    prefix,
                    catalog.hosts_view_mode,
                    collection_columns(width, false),
                    ShellMetrics::CARD_GAP,
                    children.collect(),
                )
                .into_any_element();
            }
            div()
                .w_full()
                .grid()
                .grid_cols(collection_columns(width, true))
                .gap(space(12.0))
                .children(children.map(|(_, child)| child))
                .into_any_element()
        };
        let filter = |name: String, label: String, count: usize, active: bool| {
            let controller = controller.clone();
            Button::new(SharedString::from(format!("host-filter:{name}")))
                .ghost()
                .selected(active)
                .h(space(37.0))
                .px(space(16.0))
                .rounded_full()
                .flex_shrink_0()
                .bg(if active {
                    colors.selected
                } else {
                    colors.navigation
                })
                .text_color(if active {
                    colors.on_selected
                } else {
                    colors.muted
                })
                .when(active, |button| {
                    button.child(Icon::new(AppIcon::Check).size(space(16.0)))
                })
                .child(
                    div()
                        .max_w(space(192.0))
                        .truncate()
                        .text_size(space(ShellTypography::BODY_SIZE))
                        .line_height(space(ShellTypography::BODY_LINE_HEIGHT))
                        .when(active, |label| {
                            label.font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        })
                        .child(label),
                )
                .child(
                    div()
                        .text_size(space(ShellTypography::SUPPORTING_SIZE))
                        .line_height(space(ShellTypography::SUPPORTING_LINE_HEIGHT))
                        .child(count.to_string()),
                )
                .on_click(move |_, _, cx| {
                    controller.update(cx, |controller, cx| {
                        if name.is_empty() {
                            controller.catalog_view.borrow_mut().hosts_group_filter = None;
                            cx.notify();
                        } else {
                            controller.toggle_hosts_group_filter(name.clone(), cx);
                        }
                    })
                })
        };
        let controls = h_flex()
            .id("host-group-filters")
            .w_full()
            .max_w(space(ShellMetrics::CONTENT_MAX_WIDTH))
            .mx_auto()
            .gap(space(8.0))
            .pt(space(4.0))
            .pb(space(16.0))
            .overflow_x_scroll()
            .child(filter(
                String::new(),
                i18n::string("hosts.home.all"),
                remote.len(),
                selected.is_none(),
            ))
            .children(groups.into_iter().map(|(name, count)| {
                filter(
                    name.clone(),
                    name.clone(),
                    count,
                    selected == Some(name.as_str()),
                )
            }));
        let mut content = v_flex()
            .w_full()
            .max_w(space(ShellMetrics::CONTENT_MAX_WIDTH))
            .mx_auto()
            .gap(space(24.0))
            .pb(space(112.0));
        if selected.is_none() && !self.editor_state().host_editor_open && !recent.is_empty() {
            content = content.child(
                v_flex()
                    .gap(space(12.0))
                    .child(section_heading(i18n::string("hosts.home.quick")))
                    .child(div().py(space(4.0)).child(render_collection(
                        &recent,
                        true,
                        "quick-host",
                    ))),
            );
        }
        if !favorites.is_empty() {
            content = content.child(
                v_flex()
                    .gap(space(12.0))
                    .child(section_heading(i18n::string("hosts.page.favorites")))
                    .child(render_collection(&favorites, false, "favorite-host")),
            );
        }
        let view_toggle = view_mode_switcher("hosts-view-mode", catalog.hosts_view_mode, {
            let controller = controller.clone();
            move |mode, _, cx| {
                controller.update(cx, |controller, cx| {
                    controller.set_hosts_view_mode(mode, cx)
                })
            }
        });
        let compact = width < 420.0;
        let heading = div()
            .w_full()
            .min_h(space(40.0))
            .flex()
            .gap(space(12.0))
            .when(compact, |row| row.flex_col().items_start())
            .when(!compact, |row| row.items_center().justify_between())
            .child(
                h_flex()
                    .min_w_0()
                    .max_w_full()
                    .gap(space(12.0))
                    .child(section_heading(
                        selected
                            .map(str::to_owned)
                            .unwrap_or_else(|| i18n::string("hosts.home.collection")),
                    ))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(space(12.0))
                            .line_height(space(ShellTypography::SUPPORTING_LINE_HEIGHT))
                            .px(space(8.0))
                            .py(space(4.0))
                            .rounded(space(8.0))
                            .bg(colors.navigation)
                            .text_color(colors.muted)
                            .child(i18n::string_args(
                                "hosts.home.count",
                                &[("count", &visible.len().to_string())],
                            )),
                    ),
            )
            .child(view_toggle);
        let library = if visible.is_empty() {
            let controller = controller.clone();
            v_flex()
                .items_center()
                .gap(space(12.0))
                .py(space(40.0))
                .child(section_heading(i18n::string(if remote.is_empty() {
                    "hosts.home.empty"
                } else {
                    "hosts.home.empty_group"
                })))
                .child(
                    Button::new("hosts-empty-create")
                        .primary()
                        .large()
                        .line_height(space(ShellTypography::BODY_LINE_HEIGHT))
                        .h(space(40.0))
                        .px(space(16.0))
                        .rounded(miaominal_settings::scaled_font_size(12.0))
                        .bg(colors.primary)
                        .label(i18n::string("hosts.home.create"))
                        .on_click(move |_, _, cx| {
                            controller.update(cx, |controller, cx| {
                                controller.request_new_profile_editor(cx)
                            })
                        }),
                )
                .into_any_element()
        } else {
            render_collection(&visible, false, "library-host").into_any_element()
        };
        content = content.child(v_flex().gap(space(12.0)).child(heading).child(library));
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(colors.canvas)
            .pt(space(8.0))
            .child(div().px(space(inset + 4.0)).child(controls))
            .child(
                div()
                    .id("hosts-collection-scroll")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .size_full()
                            .overflow_y_scrollbar()
                            .child(div().px(space(inset + 4.0)).pt(space(8.0)).child(content)),
                    ),
            )
            .into_any_element()
    }
}

fn section_heading(text: String) -> impl IntoElement {
    div()
        .min_w_0()
        .truncate()
        .text_size(space(ShellTypography::TITLE_SIZE))
        .line_height(space(ShellTypography::TITLE_LINE_HEIGHT))
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .text_color(ShellColors::current().foreground)
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grid_respects_card_minimum_and_quick_breakpoints() {
        assert_eq!(collection_columns(1066.0, false), 3);
        assert_eq!(collection_columns(720.0, false), 2);
        assert_eq!(collection_columns(250.0, false), 1);
        assert_eq!(collection_columns(921.0, true), 4);
        assert_eq!(collection_columns(920.0, true), 2);
        assert_eq!(collection_columns(520.0, true), 1);
    }
}
