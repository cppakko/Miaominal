use crate::ui::i18n;

use super::super::*;

pub(in crate::ui::shell::layout) fn terminal_pane_surface_id(
    pane_id: super::super::panes::PaneId,
) -> SharedString {
    SharedString::from(format!("terminal-pane-surface-{}", pane_id.raw()))
}

fn emit_terminal_menu_command(
    source: &Entity<SessionController>,
    pane_id: Option<PaneId>,
    command: TerminalMenuCommand,
    cx: &mut App,
) {
    source.update(cx, |controller, cx| {
        controller.emit(AppCommand::TerminalMenuRequested { pane_id, command }, cx);
    });
}

pub(in crate::ui::shell::layout) fn build_terminal_context_menu(
    menu: PopupMenu,
    command_source: Entity<SessionController>,
    has_selection: bool,
    target_pane_id: Option<PaneId>,
    _window: &mut Window,
    _cx: &mut App,
) -> PopupMenu {
    let copy = command_source.clone();
    let paste = command_source.clone();
    let split_right = command_source.clone();
    let split_down = command_source.clone();
    let split_left = command_source.clone();
    let split_up = command_source.clone();
    let sftp_entry = command_source.clone();
    let close = command_source;

    menu.item(
        PopupMenuItem::new(i18n::string("workspace.menu.copy"))
            .disabled(!has_selection)
            .on_click(move |_, _window, cx| {
                emit_terminal_menu_command(&copy, target_pane_id, TerminalMenuCommand::Copy, cx);
            }),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.paste")).on_click(move |_, _window, cx| {
            emit_terminal_menu_command(&paste, target_pane_id, TerminalMenuCommand::Paste, cx);
        }),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_right")).on_click(
            move |_, _window, cx| {
                emit_terminal_menu_command(
                    &split_right,
                    target_pane_id,
                    TerminalMenuCommand::Split(SplitDirection::Right),
                    cx,
                );
            },
        ),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_down")).on_click(
            move |_, _window, cx| {
                emit_terminal_menu_command(
                    &split_down,
                    target_pane_id,
                    TerminalMenuCommand::Split(SplitDirection::Down),
                    cx,
                );
            },
        ),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_left")).on_click(
            move |_, _window, cx| {
                emit_terminal_menu_command(
                    &split_left,
                    target_pane_id,
                    TerminalMenuCommand::Split(SplitDirection::Left),
                    cx,
                );
            },
        ),
    )
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.split_up")).on_click(
            move |_, _window, cx| {
                emit_terminal_menu_command(
                    &split_up,
                    target_pane_id,
                    TerminalMenuCommand::Split(SplitDirection::Up),
                    cx,
                );
            },
        ),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.open_sftp_tab")).on_click(
            move |_, _window, cx| {
                emit_terminal_menu_command(
                    &sftp_entry,
                    target_pane_id,
                    TerminalMenuCommand::OpenSftp,
                    cx,
                );
            },
        ),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("workspace.menu.close_pane")).on_click(
            move |_, _window, cx| {
                emit_terminal_menu_command(
                    &close,
                    target_pane_id,
                    TerminalMenuCommand::ClosePane,
                    cx,
                );
            },
        ),
    )
}
