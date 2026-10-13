use super::super::*;
pub(in crate::ui::shell::layout) fn advance_terminal_search_overlay(
    app: &mut AppView,
    window: &mut Window,
    cx: &App,
) -> Option<f32> {
    app.controllers
        .session
        .read(cx)
        .sample_terminal_search_overlay(window)
}
pub(in crate::ui::shell::layout) fn render_terminal_search_overlay(
    app: &AppView,
    visibility: f32,
    cx: &mut Context<AppView>,
) -> gpui_kit::AnyElement {
    let controller = app.controllers.session.clone();
    controller.read(cx).render_terminal_search_overlay(
        controller.clone(),
        app.workspace.workspace.active_pane.terminal_focus.clone(),
        visibility,
    )
}
