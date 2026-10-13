use super::*;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(in crate::ui::shell) struct DraggedTab {
    pub(in crate::ui::shell) source_tab_id: TabId,
    pub(in crate::ui::shell) source_index: usize,
    pub(in crate::ui::shell) source_pane_id: PaneId,
    pub(in crate::ui::shell) is_active: bool,
    pub(in crate::ui::shell) title: String,
    pub(in crate::ui::shell) status_color: Option<u32>,
}

impl Render for DraggedTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;

        h_flex()
            .w(px(TOPBAR_TAB_WIDTH))
            .h(px(36.0))
            .px_3()
            .gap_2()
            .items_center()
            .rounded(px(14.0))
            .bg(rgb(if self.is_active {
                roles.secondary_container
            } else {
                roles.surface_container_high
            }))
            .opacity(0.92)
            .text_size(miaominal_settings::FontSize::Body.scaled())
            .text_color(rgb(if self.is_active {
                roles.on_secondary_container
            } else {
                roles.on_surface_variant
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_2()
                    .items_center()
                    .when_some(self.status_color, |this, color| {
                        this.child(div().size(px(7.0)).rounded(px(999.0)).bg(rgb(color)))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(self.title.clone()),
                    ),
            )
    }
}

pub(in crate::ui::shell) struct ShellState {
    pub(in crate::ui::shell) sidebar_section: SidebarSection,
    pub(in crate::ui::shell) page_editor_sidebar_transition: Option<PageEditorSidebarTransition>,
    pub(in crate::ui::shell) visible_page_editor_sidebar: Option<PageEditorSidebarKind>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            sidebar_section: SidebarSection::Hosts,
            page_editor_sidebar_transition: None,
            visible_page_editor_sidebar: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_state_starts_on_ssh_profiles() {
        assert_eq!(ShellState::default().sidebar_section, SidebarSection::Hosts);
    }
}
