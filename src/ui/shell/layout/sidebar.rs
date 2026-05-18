use super::super::*;

fn sidebar_item(
    section: SidebarSection,
    label: impl Into<SharedString>,
    is_selected: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    let theme = settings::current_theme();
    let roles = theme.material.roles;

    div()
        .w_full()
        .h(px(NAV_ITEM_HEIGHT))
        .rounded(px(16.0))
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
        .items_center()
        .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, cx| {
            on_click(window, cx);
        })
        .child(
            v_flex()
                .w_full()
                .h_full()
                .items_center()
                .justify_center()
                .gap_1()
                .child(
                    div()
                        .size(px(24.0))
                        .rounded(px(8.0))
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
                .child(
                    div()
                        .text_size(settings::scaled_font_size(11.0))
                        .text_color(rgb(if is_selected {
                            roles.secondary
                        } else {
                            roles.on_surface_variant
                        }))
                        .child(label),
                ),
        )
}

impl AppView {
    pub(in crate::ui::shell) fn render_sidebar(&self, entity: Entity<Self>) -> impl IntoElement {
        let settings_selected = self.panel_view.sidebar_section == SidebarSection::Settings;
        let settings_entity = entity.clone();
        let roles = settings::current_theme().material.roles;
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
                    .px_3()
                    .py_4()
                    .children(SidebarSection::all().into_iter().map({
                        let entity = entity.clone();
                        move |section| {
                            let entity = entity.clone();
                            let is_selected = self.panel_view.sidebar_section == section;

                            sidebar_item(section, section.title(), is_selected, move |_, cx| {
                                entity.update(cx, |this, cx| this.set_sidebar_section(section, cx));
                            })
                        }
                    }))
                    .child(div().flex_1())
                    .child(sidebar_item(
                        SidebarSection::Settings,
                        SidebarSection::Settings.title(),
                        settings_selected,
                        move |_, cx| {
                            let entity = settings_entity.clone();
                            entity.update(cx, |this, cx| {
                                this.set_sidebar_section(SidebarSection::Settings, cx);
                            });
                        },
                    )),
            )
    }
}
