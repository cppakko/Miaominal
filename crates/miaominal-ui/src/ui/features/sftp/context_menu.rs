use super::*;
use gpui_kit::{DismissEvent, Subscription, anchored, deferred};
use std::rc::Rc;

pub(super) type SftpMenuBuilder =
    dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu;

#[derive(Default)]
struct MenuState {
    menu: Option<Entity<PopupMenu>>,
    position: Point<Pixels>,
    _subscription: Option<Subscription>,
}

/// DataTable 0.8 installs an inner context menu even when its delegate returns
/// no items. Observe the press before that menu consumes it, but leave row
/// selection and nested input/header menus to their existing handlers.
#[derive(IntoElement)]
pub(super) struct SftpTableContextMenu {
    id: ElementId,
    table: Entity<TableState<SftpBrowserTableDelegate>>,
    content: AnyElement,
    builder: Rc<SftpMenuBuilder>,
}

impl SftpTableContextMenu {
    pub(super) fn new(
        id: ElementId,
        table: Entity<TableState<SftpBrowserTableDelegate>>,
        content: AnyElement,
        builder: Rc<SftpMenuBuilder>,
    ) -> Self {
        Self {
            id,
            table,
            content,
            builder,
        }
    }
}

impl gpui_kit::RenderOnce for SftpTableContextMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| MenuState::default());
        let overlay = state.read(cx).menu.clone().map(|menu| {
            deferred(
                anchored().child(
                    div()
                        .w(window.bounds().size.width)
                        .h(window.bounds().size.height)
                        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                        .child(
                            anchored()
                                .position(state.read(cx).position)
                                // Match the component menu's viewport collision margin.
                                .snap_to_window_with_margin(px(8.))
                                .child(menu),
                        ),
                ),
            )
            .with_priority(gpui_kit::base::POPUP_PRIORITY)
        });
        let table = self.table;
        let builder = self.builder;

        div()
            .id(self.id)
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.content)
            .children(overlay)
            .on_mouse_down(MouseButton::Right, |_, _, cx| {
                // Loading/empty wrappers may have no inner menu to consume the
                // press. Do not also open the section's chrome context menu.
                cx.stop_propagation();
            })
            .capture_any_mouse_down(move |event, window, cx| {
                if event.button != MouseButton::Right {
                    return;
                }
                table.update(cx, |table, cx| {
                    table.delegate_mut().take_nested_context_menu_requested();
                    table.set_right_clicked_row(None, cx);
                });
                let position = event.position;
                let focus = table.focus_handle(cx);
                let state = state.downgrade();
                let builder = builder.clone();

                // The table's empty menu also defers a focus change. Build after
                // its defer and the RightClickedRow subscribers, not before them.
                window.defer(cx, move |window, cx| {
                    window.defer(cx, move |window, cx| {
                        let Some(state) = state.upgrade() else { return };
                        let menu = PopupMenu::build(window, cx, |menu, window, cx| {
                            builder(menu.action_context(focus), window, cx)
                        });
                        if menu.read(cx).is_empty() {
                            return;
                        }
                        let subscription = window.subscribe(&menu, cx, {
                            let state = state.downgrade();
                            move |_, _: &DismissEvent, window, cx| {
                                if let Some(state) = state.upgrade() {
                                    state.update(cx, |state, _| state.menu = None);
                                    window.refresh();
                                }
                            }
                        });
                        menu.focus_handle(cx).focus(window, cx);
                        state.update(cx, |state, _| {
                            state.menu = Some(menu);
                            state.position = position;
                            state._subscription = Some(subscription);
                        });
                        window.refresh();
                    });
                });
            })
    }
}
