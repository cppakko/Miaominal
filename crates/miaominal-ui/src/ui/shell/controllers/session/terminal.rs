use std::sync::Arc;

use alacritty_terminal::index::Side;
use gpui_kit::{ClipboardItem, Context};
use miaominal_ssh::SessionEventReceiver;
use miaominal_terminal::{
    MouseEncoding, MouseProtocol, TerminalEditStep, TerminalFreeTypeDropPlan,
    TerminalFreeTypeTarget, TerminalInputModes, TerminalLink, TerminalNamedKey, TerminalScroll,
    TerminalSelectionKind, TerminalSelectionPurpose, TerminalSnapshot, TerminalState,
    encode_terminal_named_key, sanitize_paste,
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

enum FreeTypeWriteResult {
    Stale,
    Complete(TerminalWriteResult),
}

enum CrossPaneSelectionDropError {
    Stale,
    Source(String),
    Target(String),
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
        kind: TerminalSelectionKind,
    ) -> bool {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return false;
        };
        session
            .terminal
            .start_selection_with_kind(line, column, side, kind);
        true
    }

    pub(in crate::ui::shell) fn terminal_free_type_target(
        &self,
        tab_id: TabId,
        line: i32,
        column: usize,
        side: Side,
    ) -> Option<TerminalFreeTypeTarget> {
        self.with_editable_terminal(tab_id, |terminal| {
            terminal.free_type_target(line, column, side)
        })
    }

    pub(in crate::ui::shell) fn start_terminal_free_type_selection(
        &self,
        tab_id: TabId,
        target: TerminalFreeTypeTarget,
        kind: TerminalSelectionKind,
    ) -> bool {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return false;
        };
        if session.is_terminal_read_only() || session.commands.is_none() {
            return false;
        }
        session
            .terminal
            .start_free_type_selection(target.line, target.column, target.side, kind)
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

    pub(in crate::ui::shell) fn terminal_has_free_type_selection(&self, tab_id: TabId) -> bool {
        self.tab(tab_id)
            .is_some_and(|session| session.terminal.has_free_type_selection())
    }

    pub(in crate::ui::shell) fn terminal_selection_contains(
        &self,
        tab_id: TabId,
        line: i32,
        column: usize,
    ) -> bool {
        self.tab(tab_id).is_some_and(|session| {
            session.terminal.has_selection() && session.terminal.selection_contains(line, column)
        })
    }

    pub(in crate::ui::shell) fn clear_terminal_free_type_selection(&self, tab_id: TabId) -> bool {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return false;
        };
        session.terminal.clear_free_type_selection()
    }

    pub(in crate::ui::shell) fn clear_all_terminal_free_type_selections(&self) -> bool {
        let mut changed = false;
        for session in self.tabs.borrow_mut().values_mut() {
            changed |= session.terminal.clear_free_type_selection();
        }
        changed
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
        self.finish_terminal_input_write(tab_id, result, cx)
    }

    fn finish_terminal_input_write(
        &mut self,
        tab_id: TabId,
        result: TerminalWriteResult,
        cx: &mut Context<Self>,
    ) -> bool {
        self.finish_terminal_write(
            tab_id,
            result,
            None,
            "session.terminal_messages.input_failed",
            cx,
        )
    }

    fn finish_terminal_write(
        &mut self,
        tab_id: TabId,
        result: TerminalWriteResult,
        sent_announce: Option<String>,
        fail_key: &'static str,
        cx: &mut Context<Self>,
    ) -> bool {
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
                if let Some(message) = sent_announce {
                    cx.emit(AppCommand::Feedback(message));
                    cx.notify();
                } else if was_scrolled {
                    cx.notify();
                }
            }
            TerminalWriteResult::Failed { error, .. } => {
                self.emit_terminal_write_failure(tab_id, fail_key, error, cx);
            }
        }
        was_scrolled
    }

    pub(in crate::ui::shell) fn send_terminal_text_input(
        &mut self,
        tab_id: TabId,
        text: String,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.terminal_has_free_type_selection(tab_id) {
            return self.send_terminal_input(tab_id, text.into_bytes(), cx);
        }

        let append =
            |bytes: &mut Vec<u8>,
             _plan: &miaominal_terminal::TerminalFreeTypePlan<Vec<TerminalEditStep>>| {
                bytes.extend_from_slice(text.as_bytes());
            };
        if let Some(handled) =
            self.send_free_type_replace(tab_id, append, Self::finish_terminal_input_write, cx)
        {
            return handled;
        }
        self.send_terminal_input(tab_id, text.into_bytes(), cx)
    }

    pub(in crate::ui::shell) fn send_terminal_free_type_cursor(
        &mut self,
        tab_id: TabId,
        target: TerminalFreeTypeTarget,
        cx: &mut Context<Self>,
    ) -> bool {
        self.send_free_type_steps(
            tab_id,
            |controller| controller.free_type_cursor_plan(tab_id, target),
            cx,
        )
    }

    pub(in crate::ui::shell) fn send_terminal_free_type_delete(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) -> bool {
        self.send_free_type_steps(
            tab_id,
            |controller| controller.free_type_delete_plan(tab_id),
            cx,
        )
    }

    pub(in crate::ui::shell) fn send_terminal_free_type_collapse(
        &mut self,
        tab_id: TabId,
        collapse_to_end: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        self.send_free_type_steps(
            tab_id,
            |controller| controller.free_type_collapse_plan(tab_id, collapse_to_end),
            cx,
        )
    }

    fn send_free_type_steps(
        &mut self,
        tab_id: TabId,
        recompute: impl Fn(
            &Self,
        )
            -> Option<miaominal_terminal::TerminalFreeTypePlan<Vec<TerminalEditStep>>>,
        cx: &mut Context<Self>,
    ) -> bool {
        for _ in 0..2 {
            let Some(plan) = recompute(self) else {
                continue;
            };
            let Some(bytes) = encode_terminal_edit_steps(&plan.value, plan.input_modes) else {
                return false;
            };
            match self.write_terminal_free_type_bytes(tab_id, plan.generation, bytes, true) {
                FreeTypeWriteResult::Stale => continue,
                FreeTypeWriteResult::Complete(result) => {
                    return self.finish_terminal_edit_write(tab_id, result, cx);
                }
            }
        }
        false
    }

    /// Replaces an active free-type selection with typed or pasted text.
    /// Returns `None` when there is nothing to replace so the caller can fall
    /// back to ordinary input, `Some(handled)` once the write is finished.
    fn send_free_type_replace(
        &mut self,
        tab_id: TabId,
        append: impl Fn(&mut Vec<u8>, &miaominal_terminal::TerminalFreeTypePlan<Vec<TerminalEditStep>>),
        finish: impl Fn(&mut Self, TabId, TerminalWriteResult, &mut Context<Self>) -> bool,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        for _ in 0..2 {
            let plan = self.free_type_replace_prefix(tab_id)?;
            let Some(mut bytes) = encode_terminal_edit_steps(&plan.value, plan.input_modes) else {
                return Some(false);
            };
            append(&mut bytes, &plan);
            match self.write_terminal_free_type_bytes(tab_id, plan.generation, bytes, true) {
                FreeTypeWriteResult::Stale => continue,
                FreeTypeWriteResult::Complete(result) => {
                    return Some(finish(self, tab_id, result, cx));
                }
            }
        }
        self.clear_terminal_free_type_selection(tab_id);
        None
    }

    pub(in crate::ui::shell) fn send_terminal_selection_drop(
        &mut self,
        tab_id: TabId,
        target: TerminalFreeTypeTarget,
        duplicate: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        for _ in 0..2 {
            let Some((plan, clear_selection)) = self.selection_drop_plan(tab_id, target, duplicate)
            else {
                continue;
            };
            let Some(mut bytes) = encode_terminal_edit_steps(&plan.value.steps, plan.input_modes)
            else {
                return false;
            };
            bytes.extend(sanitize_paste(
                &plan.value.text,
                plan.input_modes.bracketed_paste,
            ));
            match self.write_terminal_free_type_bytes(
                tab_id,
                plan.generation,
                bytes,
                clear_selection,
            ) {
                FreeTypeWriteResult::Stale => continue,
                FreeTypeWriteResult::Complete(result) => {
                    return self.finish_terminal_edit_write(tab_id, result, cx);
                }
            }
        }
        false
    }

    pub(in crate::ui::shell) fn send_terminal_selection_cross_pane_drop(
        &mut self,
        source_tab_id: TabId,
        target_tab_id: TabId,
        target: TerminalFreeTypeTarget,
        duplicate: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if source_tab_id == target_tab_id {
            return self.send_terminal_selection_drop(source_tab_id, target, duplicate, cx);
        }

        for _ in 0..2 {
            let Some(mut source_terminal) = self
                .tab(source_tab_id)
                .map(|session| session.terminal.clone())
            else {
                return false;
            };
            let Some((mut target_terminal, target_commands)) =
                self.tab(target_tab_id).and_then(|session| {
                    if session.is_terminal_read_only() {
                        return None;
                    }
                    Some((session.terminal.clone(), session.commands.clone()?))
                })
            else {
                return false;
            };

            let Some(source_plan) = source_terminal.selection_drag_plan() else {
                continue;
            };
            if source_plan.value.text.is_empty() {
                return false;
            }
            let delete_source = !duplicate && source_plan.value.delete_steps.is_some();
            let source_commands = if delete_source {
                let Some(commands) = self.tab(source_tab_id).and_then(|session| {
                    (!session.is_terminal_read_only())
                        .then(|| session.commands.clone())
                        .flatten()
                }) else {
                    return false;
                };
                Some(commands)
            } else {
                None
            };
            let Some(target_plan) =
                target_terminal.free_type_cursor_plan(target.line, target.column, target.side)
            else {
                continue;
            };
            let Some(mut target_bytes) =
                encode_terminal_edit_steps(&target_plan.value, target_plan.input_modes)
            else {
                return false;
            };
            target_bytes.extend(sanitize_paste(
                &source_plan.value.text,
                target_plan.input_modes.bracketed_paste,
            ));
            let source_delete_bytes =
                if delete_source {
                    let Some(bytes) = source_plan.value.delete_steps.as_ref().and_then(|steps| {
                        encode_terminal_edit_steps(steps, source_plan.input_modes)
                    }) else {
                        return false;
                    };
                    Some(bytes)
                } else {
                    None
                };
            let target_len = target_bytes.len() as u64;
            let source_len = source_delete_bytes
                .as_ref()
                .map_or(0, |bytes| bytes.len() as u64);
            let mut target_sent = false;
            let mut source_deleted = false;

            let result = source_terminal.commit_free_type_plan(
                source_plan.generation,
                delete_source,
                || {
                    let target_result = target_terminal.commit_free_type_plan(
                        target_plan.generation,
                        true,
                        || {
                            target_commands.send_bytes(target_bytes).map_err(|error| {
                                CrossPaneSelectionDropError::Target(error.to_string())
                            })
                        },
                    )?;
                    if target_result.is_none() {
                        return Err(CrossPaneSelectionDropError::Stale);
                    }
                    target_sent = true;

                    if let (Some(source_commands), Some(source_delete_bytes)) =
                        (source_commands, source_delete_bytes)
                    {
                        source_commands
                            .send_bytes(source_delete_bytes)
                            .map_err(|error| {
                                CrossPaneSelectionDropError::Source(error.to_string())
                            })?;
                        source_deleted = true;
                    }
                    Ok(())
                },
            );

            if target_sent && let Some(mut session) = self.tab_mut(target_tab_id) {
                session.bytes_out = session.bytes_out.saturating_add(target_len);
            }
            if source_deleted && let Some(mut session) = self.tab_mut(source_tab_id) {
                session.bytes_out = session.bytes_out.saturating_add(source_len);
            }

            match result {
                Ok(None) | Err(CrossPaneSelectionDropError::Stale) => continue,
                Ok(Some(())) => {
                    cx.notify();
                    return true;
                }
                Err(CrossPaneSelectionDropError::Source(error)) => {
                    if let Some(mut session) = self.tab_mut(source_tab_id) {
                        session.set_connection_state(SessionConnectionState::Disconnected);
                    }
                    self.emit_terminal_write_failure(
                        source_tab_id,
                        "session.terminal_messages.input_failed",
                        error,
                        cx,
                    );
                    return false;
                }
                Err(CrossPaneSelectionDropError::Target(error)) => {
                    if let Some(mut session) = self.tab_mut(target_tab_id) {
                        session.set_connection_state(SessionConnectionState::Disconnected);
                    }
                    self.emit_terminal_write_failure(
                        target_tab_id,
                        "session.terminal_messages.input_failed",
                        error,
                        cx,
                    );
                    return false;
                }
            }
        }
        false
    }

    pub(in crate::ui::shell) fn paste_terminal_text(
        &mut self,
        tab_id: TabId,
        text: String,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.terminal_has_free_type_selection(tab_id) {
            let append = |bytes: &mut Vec<u8>,
                          plan: &miaominal_terminal::TerminalFreeTypePlan<
                Vec<TerminalEditStep>,
            >| {
                bytes.extend(sanitize_paste(&text, plan.input_modes.bracketed_paste));
            };
            let finish = |this: &mut Self,
                          tab_id: TabId,
                          result: TerminalWriteResult,
                          cx: &mut Context<Self>| {
                this.finish_terminal_paste_write(tab_id, &text, result, cx)
            };
            if let Some(handled) = self.send_free_type_replace(tab_id, append, finish, cx) {
                return handled;
            }
        }

        let Some(bracketed) = self
            .tab(tab_id)
            .map(|session| session.terminal.bracketed_paste_enabled())
        else {
            return false;
        };
        let bytes = sanitize_paste(&text, bracketed);
        let result = self.write_terminal_bytes(tab_id, bytes, true);
        self.finish_terminal_paste_write(tab_id, &text, result, cx)
    }

    fn finish_terminal_paste_write(
        &mut self,
        tab_id: TabId,
        text: &str,
        result: TerminalWriteResult,
        cx: &mut Context<Self>,
    ) -> bool {
        let count = text.chars().count().to_string();
        let sent_announce = Some(i18n::string_args(
            "session.terminal_messages.pasted_characters",
            &[("count", &count)],
        ));
        self.finish_terminal_write(
            tab_id,
            result,
            sent_announce,
            "session.terminal_messages.paste_failed",
            cx,
        )
    }

    fn with_editable_terminal<T>(
        &self,
        tab_id: TabId,
        f: impl FnOnce(&TerminalState) -> Option<T>,
    ) -> Option<T> {
        let session = self.tab(tab_id)?;
        if session.is_terminal_read_only() || session.commands.is_none() {
            return None;
        }
        f(&session.terminal)
    }

    fn free_type_replace_prefix(
        &self,
        tab_id: TabId,
    ) -> Option<miaominal_terminal::TerminalFreeTypePlan<Vec<TerminalEditStep>>> {
        self.with_editable_terminal(tab_id, TerminalState::free_type_delete_plan)
    }

    fn free_type_cursor_plan(
        &self,
        tab_id: TabId,
        target: TerminalFreeTypeTarget,
    ) -> Option<miaominal_terminal::TerminalFreeTypePlan<Vec<TerminalEditStep>>> {
        self.with_editable_terminal(tab_id, |terminal| {
            terminal.free_type_cursor_plan(target.line, target.column, target.side)
        })
    }

    fn free_type_delete_plan(
        &self,
        tab_id: TabId,
    ) -> Option<miaominal_terminal::TerminalFreeTypePlan<Vec<TerminalEditStep>>> {
        self.with_editable_terminal(tab_id, TerminalState::free_type_delete_plan)
    }

    fn free_type_collapse_plan(
        &self,
        tab_id: TabId,
        collapse_to_end: bool,
    ) -> Option<miaominal_terminal::TerminalFreeTypePlan<Vec<TerminalEditStep>>> {
        self.with_editable_terminal(tab_id, |terminal| {
            terminal.free_type_collapse_plan(collapse_to_end)
        })
    }

    fn selection_drop_plan(
        &self,
        tab_id: TabId,
        target: TerminalFreeTypeTarget,
        duplicate: bool,
    ) -> Option<(
        miaominal_terminal::TerminalFreeTypePlan<TerminalFreeTypeDropPlan>,
        bool,
    )> {
        let clear_selection = self.tab(tab_id)?.terminal.selection_purpose()
            == Some(TerminalSelectionPurpose::FreeType)
            && !duplicate;
        let plan = self.with_editable_terminal(tab_id, |terminal| {
            terminal.selection_drop_plan(target.line, target.column, target.side, duplicate)
        })?;
        Some((plan, clear_selection))
    }

    fn finish_terminal_edit_write(
        &mut self,
        tab_id: TabId,
        result: TerminalWriteResult,
        cx: &mut Context<Self>,
    ) -> bool {
        let success = matches!(result, TerminalWriteResult::Sent { .. });
        self.finish_terminal_input_write(tab_id, result, cx);
        if success {
            cx.notify();
        }
        success
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

    fn write_terminal_free_type_bytes(
        &self,
        tab_id: TabId,
        generation: u64,
        bytes: Vec<u8>,
        clear_selection: bool,
    ) -> FreeTypeWriteResult {
        let Some(mut session) = self.tab_mut(tab_id) else {
            return FreeTypeWriteResult::Complete(TerminalWriteResult::Missing);
        };
        if session.is_terminal_read_only() {
            return FreeTypeWriteResult::Complete(TerminalWriteResult::ReadOnly);
        }
        let Some(commands) = session.commands.clone() else {
            return FreeTypeWriteResult::Complete(TerminalWriteResult::Connecting);
        };

        let len = bytes.len() as u64;
        let committed = session
            .terminal
            .commit_free_type_plan(generation, clear_selection, || {
                if bytes.is_empty() {
                    Ok(())
                } else {
                    commands.send_bytes(bytes)
                }
            });
        match committed {
            Ok(None) => FreeTypeWriteResult::Stale,
            Ok(Some(())) => {
                session.bytes_out = session.bytes_out.saturating_add(len);
                FreeTypeWriteResult::Complete(TerminalWriteResult::Sent {
                    was_scrolled: false,
                })
            }
            Err(error) => {
                session.set_connection_state(SessionConnectionState::Disconnected);
                FreeTypeWriteResult::Complete(TerminalWriteResult::Failed {
                    was_scrolled: false,
                    error: error.to_string(),
                })
            }
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

fn encode_terminal_edit_steps(
    steps: &[TerminalEditStep],
    input_modes: TerminalInputModes,
) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    for step in steps {
        let key = match step {
            TerminalEditStep::Left(_) => TerminalNamedKey::Left,
            TerminalEditStep::Right(_) => TerminalNamedKey::Right,
            TerminalEditStep::Up(_) => TerminalNamedKey::Up,
            TerminalEditStep::Down(_) => TerminalNamedKey::Down,
            TerminalEditStep::Delete(_) => TerminalNamedKey::Delete,
        };
        let encoded = encode_terminal_named_key(key, input_modes)?;
        for _ in 0..step.count() {
            bytes.extend_from_slice(&encoded);
        }
    }
    Some(bytes)
}
