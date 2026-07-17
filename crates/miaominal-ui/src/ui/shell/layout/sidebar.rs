use super::super::*;

fn sidebar_item(
    section: SidebarSection,
    is_selected: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let theme = miaominal_settings::current_theme();
    let roles = theme.material.roles;
    let hover_background = if is_selected {
        roles.secondary_container
    } else {
        roles.surface_container_highest
    };
    let item_id = match section {
        SidebarSection::Hosts => "sidebar-hosts",
        SidebarSection::Keychain => "sidebar-keychain",
        SidebarSection::PortForwarding => "sidebar-forwarding",
        SidebarSection::Snippets => "sidebar-snippets",
        SidebarSection::KnownHosts => "sidebar-known-hosts",
        SidebarSection::Settings => "sidebar-settings",
    };

    div()
        .id(item_id)
        .w_full()
        .h(px(NAV_ITEM_HEIGHT))
        .rounded(px(14.0))
        .border_color(rgb(if is_selected {
            roles.outline
        } else {
            roles.outline_variant
        }))
        .bg(rgb(if is_selected {
            roles.secondary_container
        } else {
            roles.surface_container
        }))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |this| this.bg(rgb(hover_background)))
        .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, cx| {
            on_click(window, cx);
        })
        .child(
            div()
                .size(px(28.0))
                .rounded(px(10.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(if is_selected {
                    roles.on_secondary_container
                } else {
                    roles.on_surface_variant
                }))
                .child(section.icon()),
        )
}

pub(in crate::ui::shell) fn render_sidebar(
    app: &AppView,
    command_source: Entity<SettingsController>,
) -> impl IntoElement {
    let settings_selected = app.shell.shell_state.sidebar_section == SidebarSection::Settings;
    let settings_entity = command_source.clone();
    let roles = miaominal_settings::current_theme().material.roles;
    div()
        .w(px(LEFT_RAIL_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(rgb(roles.surface_container))
        .border_r_1()
        .child(
            v_flex()
                .size_full()
                .gap_4()
                .px_2()
                .py_4()
                .children(SidebarSection::all().into_iter().map({
                    let entity = command_source.clone();
                    move |section| {
                        let entity = entity.clone();
                        let is_selected = app.shell.shell_state.sidebar_section == section;

                        sidebar_item(section, is_selected, move |_, cx| {
                            entity.update(cx, |controller, cx| {
                                controller.emit(AppCommand::SidebarSectionRequested(section), cx);
                            });
                        })
                    }
                }))
                .child(div().flex_1())
                .child(sidebar_item(
                    SidebarSection::Settings,
                    settings_selected,
                    move |_, cx| {
                        let entity = settings_entity.clone();
                        entity.update(cx, |controller, cx| {
                            controller.emit(
                                AppCommand::SidebarSectionRequested(SidebarSection::Settings),
                                cx,
                            );
                        });
                    },
                )),
        )
}
