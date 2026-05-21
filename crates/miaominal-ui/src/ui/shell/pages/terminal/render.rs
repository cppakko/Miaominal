use super::super::super::*;

impl AppView {
    pub(in crate::ui::shell) fn render_terminal_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.render_workspace_surface(window, cx)
    }
}
