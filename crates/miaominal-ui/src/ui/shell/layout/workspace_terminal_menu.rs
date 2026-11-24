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

pub(in crate::ui::shell::layout) struct TerminalContextMenuState {
    pub(in crate::ui::shell::layout) has_selection: bool,
    pub(in crate::ui::shell::layout) allow_sftp: bool,
    pub(in crate::ui::shell::layout) is_split: bool,
    pub(in crate::ui::shell::layout) is_zoomed: bool,
    pub(in crate::ui::shell::layout) sync_panes: bool,
    pub(in crate::ui::shell::layout) cwd_sync_path: Option<String>,
}

pub(in crate::ui::shell::layout) fn build_terminal_context_menu(
    menu: PopupMenu,
    command_source: Entity<SessionController>,
    state: TerminalContextMenuState,
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
    let zoom_pane = command_source.clone();
    let sync_panes = command_source.clone();
    let sftp_entry = command_source.clone();
    let sftp_cwd = command_source.clone();
    let close = command_source;

    let menu = menu
        .item(
            PopupMenuItem::new(i18n::string("workspace.menu.copy"))
                .disabled(!state.has_selection)
                .on_click(move |_, _window, cx| {
                    emit_terminal_menu_command(
                        &copy,
                        target_pane_id,
                        TerminalMenuCommand::Copy,
                        cx,
                    );
                }),
        )
        .item(
            PopupMenuItem::new(i18n::string("workspace.menu.paste")).on_click(
                move |_, _window, cx| {
                    emit_terminal_menu_command(
                        &paste,
                        target_pane_id,
                        TerminalMenuCommand::Paste,
                        cx,
                    );
                },
            ),
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
        );
    let menu = if state.is_split {
        menu.item(PopupMenuItem::separator())
            .item(
                PopupMenuItem::new(i18n::string(if state.is_zoomed {
                    "session.workspace.unzoom_pane"
                } else {
                    "session.workspace.zoom_pane"
                }))
                .on_click(move |_, _window, cx| {
                    emit_terminal_menu_command(
                        &zoom_pane,
                        target_pane_id,
                        TerminalMenuCommand::ToggleZoomPane,
                        cx,
                    );
                }),
            )
            .item(
                PopupMenuItem::new(i18n::string(if state.sync_panes {
                    "session.workspace.disable_sync_panes"
                } else {
                    "session.workspace.enable_sync_panes"
                }))
                .checked(state.sync_panes)
                .on_click(move |_, _window, cx| {
                    emit_terminal_menu_command(
                        &sync_panes,
                        target_pane_id,
                        TerminalMenuCommand::ToggleSyncPanes,
                        cx,
                    );
                }),
            )
    } else {
        menu
    };
    let menu = if state.allow_sftp {
        let menu = menu.item(PopupMenuItem::separator()).item(
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
        );
        if let Some(path) = state.cwd_sync_path {
            menu.item(
                PopupMenuItem::new(i18n::string_args(
                    "session.workspace.sync_sftp_cwd",
                    &[("path", path.as_str())],
                ))
                .on_click(move |_, _window, cx| {
                    emit_terminal_menu_command(
                        &sftp_cwd,
                        target_pane_id,
                        TerminalMenuCommand::SyncSftpCwd,
                        cx,
                    );
                }),
            )
        } else {
            menu
        }
    } else {
        menu
    };
    menu.item(PopupMenuItem::separator()).item(
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
