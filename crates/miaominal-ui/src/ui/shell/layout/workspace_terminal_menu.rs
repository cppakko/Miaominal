use crate::ui::i18n;

use super::super::*;

pub(in crate::ui::shell::layout) fn terminal_pane_surface_id(
    pane_id: super::super::panes::PaneId,
) -> SharedString {
    SharedString::from(format!("terminal-pane-surface-{}", pane_id.raw()))
}

fn activate_terminal_menu_target(
    this: &mut AppView,
    target_pane_id: Option<super::super::panes::PaneId>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) {
    if let Some(pane_id) = target_pane_id {
        this.set_active_pane(pane_id, window, cx);
    }
}

pub(in crate::ui::shell::layout) fn build_terminal_context_menu(
    menu: PopupMenu,
    entity: Entity<AppView>,
    has_selection: bool,
    target_pane_id: Option<super::super::panes::PaneId>,
    _window: &mut Window,
    _cx: &mut App,
) -> PopupMenu {
    let copy = entity.clone();
    let paste = entity.clone();
    let split_right = entity.clone();
    let split_down = entity.clone();
    let split_left = entity.clone();
    let split_up = entity.clone();
    let sftp_entry = entity.clone();
    let close = entity.clone();

    menu.item(
        PopupMenuItem::new(i18n::string("workspace.menu.copy"))
            .disabled(!has_selection)
            .on_click(move |_, window, cx| {
                let entity = copy.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.copy_terminal_selection(cx);
                    window.focus(
                        &this.workspace_state.workspace.active_pane.terminal_focus,
                        cx,
                    );
                    this.sync_terminal_focus_reporting(window, cx);
                });
            }),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.paste")).on_click(move |_, window, cx| {
            let entity = paste.clone();
            entity.update(cx, |this, cx| {
                activate_terminal_menu_target(this, target_pane_id, window, cx);
                this.paste_into_terminal(cx);
                window.focus(
                    &this.workspace_state.workspace.active_pane.terminal_focus,
                    cx,
                );
                this.sync_terminal_focus_reporting(window, cx);
            });
        }),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_right")).on_click(
            move |_, window, cx| {
                let entity = split_right.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.split_active_pane(
                        super::super::workspace::SplitDirection::Right,
                        window,
                        cx,
                    )
                });
            },
        ),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_down")).on_click(
            move |_, window, cx| {
                let entity = split_down.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.split_active_pane(
                        super::super::workspace::SplitDirection::Down,
                        window,
                        cx,
                    )
                });
            },
        ),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_left")).on_click(
            move |_, window, cx| {
                let entity = split_left.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.split_active_pane(
                        super::super::workspace::SplitDirection::Left,
                        window,
                        cx,
                    )
                });
            },
        ),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_up")).on_click(
            move |_, window, cx| {
                let entity = split_up.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.split_active_pane(super::super::workspace::SplitDirection::Up, window, cx)
                });
            },
        ),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.open_sftp_tab")).on_click(
            move |_, window, cx| {
                let entity = sftp_entry.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.open_sftp_tab_for_session(
                        this.workspace_state.workspace.active_tab,
                        window,
                        cx,
                    )
                });
            },
        ),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.close_pane")).on_click(
            move |_, window, cx| {
                let entity = close.clone();
                entity.update(cx, |this, cx| {
                    activate_terminal_menu_target(this, target_pane_id, window, cx);
                    this.close_active_pane(window, cx)
                });
            },
        ),
    )
}
