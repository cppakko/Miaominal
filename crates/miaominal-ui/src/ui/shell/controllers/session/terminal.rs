use std::sync::Arc;

use alacritty_terminal::index::Side;
use gpui::{ClipboardItem, Context};
use miaominal_ssh::SessionEventReceiver;
use miaominal_terminal::{
    MouseEncoding, MouseProtocol, TerminalInputModes, TerminalLink, TerminalScroll,
    TerminalSnapshot, TerminalState, sanitize_paste,
};

use super::{SessionConnectionState, SessionController};
use crate::ui::{
    i18n,
    shell::{AppCommand, TabId},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct TerminalViewportState {
    pub(in crate::ui::shell) columns: usize,
    pub(in crate::ui::shell) screen_lines: usize,
    pub(in crate::ui::shell) display_offset: usize,
    pub(in crate::ui::shell) history_size: usize,
    pub(in crate::ui::shell) generation: u64,
}

enum TerminalWriteResult {
    Missing,
    ReadOnly,
    Connecting,
    Sent { was_scrolled: bool },
    Failed { was_scrolled: bool, error: String },
}

impl TerminalWriteResult {
    fn was_scrolled(&self) -> bool {
        matches!(
            self,
            Self::Sent { was_scrolled: true }
                | Self::Failed {
                    was_scrolled: true,
                    ..
                }
        )
    }
}

impl SessionController {
    pub(in crate::ui::shell) fn terminal_snapshot(
        &self,
        tab_id: TabId,
        focused: bool,
    ) -> Option<Arc<TerminalSnapshot>> {
        self.tab(tab_id)
            .map(|session| session.terminal.snapshot(focused))
    }

    pub(in crate::ui::shell) fn terminal_state(&self, tab_id: TabId) -> Option<TerminalState> {
        self.tab(tab_id).map(|session| session.terminal.clone())
    }

    pub(in crate::ui::shell) fn terminal_viewport_state(
        &self,
        tab_id: TabId,
    ) -> Option<TerminalViewportState> {
        let session = self.tab(tab_id)?;
        Some(TerminalViewportState {
            columns: session.terminal.columns(),
            screen_lines: session.terminal.screen_lines(),
            display_offset: session.terminal.display_offset(),
            history_size: session.terminal.history_size(),
            generation: session.terminal.generation(),
        })
    }

    pub(in crate::ui::shell) fn terminal_link_at(
        &self,
        tab_id: TabId,
        line: usize,
        column: usize,
    ) -> Option<TerminalLink> {
        self.tab(tab_id)?.terminal.link_at(line, column)
    }

    pub(in crate::ui::shell) fn terminal_input_modes(
        &self,
        tab_id: TabId,
    ) -> Option<TerminalInputModes> {
        self.tab(tab_id)
            .map(|session| session.terminal.input_modes())
    }

    pub(in crate::ui::shell) fn terminal_mouse_mode(
        &self,
        tab_id: TabId,
    ) -> Option<(MouseProtocol, MouseEncoding)> {
        let session = self.tab(tab_id)?;
        Some((
            session.terminal.mouse_protocol(),
            session.terminal.mouse_encoding(),
        ))
    }

    pub(in crate::ui::shell) fn terminal_alternate_scroll_active(&self, tab_id: TabId) -> bool {
        self.tab(tab_id)
            .is_some_and(|session| session.terminal.alternate_scroll_active())
    }

    pub(in crate::ui::shell) fn scroll_terminal(
        &self,
        tab_id: TabId,
        scroll: TerminalScroll,
    ) -> bool {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return false;
        };
        session.terminal.scroll(scroll);
        true
    }

    pub(in crate::ui::shell) fn resize_terminal_for_viewport(
        &self,
        tab_id: TabId,
        columns: usize,
        lines: usize,
        bounds_known: bool,
        allow_pending_start: bool,
    ) -> (bool, Option<SessionEventReceiver>) {
        let (size_changed, monitoring_enabled, profile_id, pending_profile, live_commands) = {
            let Some(mut session) = self.tab_mut(tab_id) else {
                return (false, None);
            };

            let size_changed = session.terminal.resize(columns, lines);
            let monitoring_enabled = session.monitoring.auto_collect_enabled;
            let profile_id = session.profile_id.clone();
            let mut pending_profile = None;
            let mut live_commands = None;
            if bounds_known && session.commands.is_none() {
                if allow_pending_start {
                    pending_profile = session.pending_profile.take();
                }
            } else if size_changed {
                live_commands = session.commands.clone();
            }

            (
                size_changed,
                monitoring_enabled,
                profile_id,
                pending_profile,
                live_commands,
            )
        };
        let monitoring_enabled =
            self.claim_profile_monitor_source(&profile_id, tab_id, monitoring_enabled);

        if let Some(profile) = pending_profile {
            let connection =
                self.start_terminal_session(profile, columns, lines, monitoring_enabled);
            if let Some(mut session) = self.tab_mut(tab_id) {
                session.commands = Some(connection.commands);
                return (true, Some(connection.events));
            }
            return (size_changed, None);
        }

        if let Some(commands) = live_commands
            && let Err(error) = commands.resize(columns, lines)
        {
            log::debug!("failed to resize remote PTY: {error:?}");
        }

        (size_changed, None)
    }

    pub(in crate::ui::shell) fn scroll_terminal_to_display_offset(
        &self,
        tab_id: TabId,
        target_offset: usize,
    ) -> bool {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return false;
        };
        session.terminal.scroll_to_display_offset(target_offset);
        true
    }

    pub(in crate::ui::shell) fn scroll_terminal_display_offset_by(
        &self,
        tab_id: TabId,
        delta: i32,
    ) -> bool {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return false;
        };
        let current_offset = session.terminal.display_offset() as i32;
        let target_offset = (current_offset + delta).max(0) as usize;
        session.terminal.scroll_to_display_offset(target_offset);
        true
    }

    pub(in crate::ui::shell) fn start_terminal_selection(
        &self,
        tab_id: TabId,
        line: i32,
        column: usize,
        side: Side,
        block: bool,
    ) -> bool {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return false;
        };
        session.terminal.start_selection(line, column, side, block);
        true
    }

    pub(in crate::ui::shell) fn update_terminal_selection(
        &self,
        tab_id: TabId,
        line: i32,
        column: usize,
        side: Side,
    ) -> bool {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return false;
        };
        session.terminal.update_selection(line, column, side);
        true
    }

    pub(in crate::ui::shell) fn clear_terminal_selection(&self, tab_id: TabId) -> bool {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return false;
        };
        if !session.terminal.has_selection() {
            return false;
        }
        session.terminal.clear_selection();
        true
    }

    pub(in crate::ui::shell) fn terminal_has_selection(&self, tab_id: TabId) -> bool {
        self.tab(tab_id)
            .is_some_and(|session| session.terminal.has_selection())
    }

    pub(in crate::ui::shell) fn terminal_selection_text(&self, tab_id: TabId) -> Option<String> {
        self.tab(tab_id)?.terminal.selection_text()
    }

    pub(in crate::ui::shell) fn copy_terminal_selection(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) -> bool {
        let text = self
            .terminal_selection_text(tab_id)
            .filter(|text| !text.is_empty());
        let Some(text) = text else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "session.terminal_messages.no_selection_to_copy",
            )));
            cx.notify();
            return false;
        };

        let length = text.chars().count().to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "session.terminal_messages.copied_characters",
            &[("count", &length)],
        )));
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn paste_terminal_clipboard(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "session.terminal_messages.clipboard_empty",
            )));
            cx.notify();
            return false;
        };

        self.paste_terminal_text(tab_id, text, cx)
    }

    pub(in crate::ui::shell) fn paste_terminal_selection_or_clipboard(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) -> bool {
        let text = self
            .terminal_selection_text(tab_id)
            .filter(|text| !text.is_empty())
            .or_else(|| cx.read_from_clipboard().and_then(|item| item.text()));
        let Some(text) = text else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "session.terminal_messages.nothing_to_paste",
            )));
            cx.notify();
            return false;
        };

        self.paste_terminal_text(tab_id, text, cx)
    }

    pub(in crate::ui::shell) fn send_terminal_input(
        &mut self,
        tab_id: TabId,
        bytes: Vec<u8>,
        cx: &mut Context<Self>,
    ) -> bool {
        let result = self.write_terminal_bytes(tab_id, bytes, true);
        let was_scrolled = result.was_scrolled();
        if was_scrolled {
            cx.emit(AppCommand::TerminalScrolledToBottom(tab_id));
        }
        match result {
            TerminalWriteResult::Missing => {}
            TerminalWriteResult::ReadOnly => {
                cx.emit(AppCommand::Feedback(i18n::string(
                    "session.terminal_messages.read_only_history",
                )));
                cx.notify();
            }
            TerminalWriteResult::Connecting => {
                cx.emit(AppCommand::Feedback(i18n::string(
                    "session.terminal_messages.connection_starting",
                )));
                cx.notify();
            }
            TerminalWriteResult::Sent { .. } => {
                if was_scrolled {
                    cx.notify();
                }
            }
            TerminalWriteResult::Failed { error, .. } => {
                self.emit_terminal_write_failure(
                    tab_id,
                    "session.terminal_messages.input_failed",
                    error,
                    cx,
                );
            }
        }
        was_scrolled
    }

    pub(in crate::ui::shell) fn paste_terminal_text(
        &mut self,
        tab_id: TabId,
        text: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(bracketed) = self
            .tab(tab_id)
            .map(|session| session.terminal.bracketed_paste_enabled())
        else {
            return false;
        };
        let bytes = sanitize_paste(&text, bracketed);
        let result = self.write_terminal_bytes(tab_id, bytes, true);
        let was_scrolled = result.was_scrolled();
        if was_scrolled {
            cx.emit(AppCommand::TerminalScrolledToBottom(tab_id));
        }
        match result {
            TerminalWriteResult::Missing => {}
            TerminalWriteResult::ReadOnly => {
                cx.emit(AppCommand::Feedback(i18n::string(
                    "session.terminal_messages.read_only_history",
                )));
                cx.notify();
            }
            TerminalWriteResult::Connecting => {
                cx.emit(AppCommand::Feedback(i18n::string(
                    "session.terminal_messages.connection_starting",
                )));
                cx.notify();
            }
            TerminalWriteResult::Sent { .. } => {
                let count = text.chars().count().to_string();
                cx.emit(AppCommand::Feedback(i18n::string_args(
                    "session.terminal_messages.pasted_characters",
                    &[("count", &count)],
                )));
                cx.notify();
            }
            TerminalWriteResult::Failed { error, .. } => {
                self.emit_terminal_write_failure(
                    tab_id,
                    "session.terminal_messages.paste_failed",
                    error,
                    cx,
                );
            }
        }
        was_scrolled
    }

    pub(in crate::ui::shell) fn send_terminal_mouse_report(
        &mut self,
        tab_id: TabId,
        bytes: Vec<u8>,
        cx: &mut Context<Self>,
    ) {
        match self.write_terminal_bytes(tab_id, bytes, false) {
            TerminalWriteResult::Missing | TerminalWriteResult::ReadOnly => {}
            TerminalWriteResult::Connecting => {
                cx.emit(AppCommand::Feedback(i18n::string(
                    "session.terminal_messages.connection_starting",
                )));
                cx.notify();
            }
            TerminalWriteResult::Sent { .. } => {}
            TerminalWriteResult::Failed { error, .. } => self.emit_terminal_write_failure(
                tab_id,
                "session.terminal_messages.mouse_report_failed",
                error,
                cx,
            ),
        }
    }

    pub(in crate::ui::shell) fn send_terminal_focus_report(
        &mut self,
        tab_id: TabId,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let can_report = self.tab(tab_id).is_some_and(|session| {
            !session.is_terminal_read_only()
                && session.terminal.input_modes().focus_in_out
                && session.commands.is_some()
        });
        if !can_report {
            return false;
        }

        let bytes = if focused {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        };
        match self.write_terminal_bytes(tab_id, bytes, false) {
            TerminalWriteResult::Sent { .. } => true,
            TerminalWriteResult::Failed { error, .. } => {
                self.emit_terminal_write_failure(
                    tab_id,
                    "session.terminal_messages.focus_report_failed",
                    error,
                    cx,
                );
                false
            }
            TerminalWriteResult::Missing
            | TerminalWriteResult::ReadOnly
            | TerminalWriteResult::Connecting => false,
        }
    }

    fn write_terminal_bytes(
        &self,
        tab_id: TabId,
        bytes: Vec<u8>,
        scroll_to_bottom: bool,
    ) -> TerminalWriteResult {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return TerminalWriteResult::Missing;
        };
        if session.is_terminal_read_only() {
            return TerminalWriteResult::ReadOnly;
        }
        let Some(commands) = session.commands.clone() else {
            return TerminalWriteResult::Connecting;
        };

        let was_scrolled = scroll_to_bottom && session.terminal.display_offset() != 0;
        if scroll_to_bottom {
            session.terminal.scroll_to_bottom();
        }
        let len = bytes.len() as u64;
        if let Err(error) = commands.send_bytes(bytes) {
            session.set_connection_state(SessionConnectionState::Disconnected);
            TerminalWriteResult::Failed {
                was_scrolled,
                error: error.to_string(),
            }
        } else {
            session.bytes_out = session.bytes_out.saturating_add(len);
            TerminalWriteResult::Sent { was_scrolled }
        }
    }

    fn emit_terminal_write_failure(
        &self,
        tab_id: TabId,
        message_key: &'static str,
        error: String,
        cx: &mut Context<Self>,
    ) {
        cx.emit(AppCommand::TabStatusChanged {
            tab_id,
            status: i18n::string("session.status.disconnected"),
        });
        cx.emit(AppCommand::Feedback(i18n::string_args(
            message_key,
            &[("error", &error)],
        )));
        cx.notify();
    }
}
