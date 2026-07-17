use std::time::Instant;

use gpui::{Context, FocusHandle, Window};

use super::SessionController;
use crate::ui::{
    i18n,
    shell::{OVERLAY_ENTER_DURATION, TabId, TerminalSearchAnimation, set_input_value},
};

impl SessionController {
    pub(in crate::ui::shell) fn sync_terminal_search_target(&self, tab_id: Option<TabId>) {
        if !self.terminal_search_open() {
            return;
        }
        let tab_id = tab_id.filter(|tab_id| self.tab(*tab_id).is_some());
        self.search_target.set(tab_id);
    }

    pub(in crate::ui::shell) fn open_terminal_search(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab(tab_id).is_none() {
            return;
        }
        self.search_target.set(Some(tab_id));
        let search_input = self.terminal_search_input();
        {
            let mut search = self.terminal_search_mut();
            search.open = true;
            search.visible = true;
            search.animation = Some(TerminalSearchAnimation {
                started_at: Instant::now(),
                duration: OVERLAY_ENTER_DURATION,
                from: search.visibility,
                to: 1.0,
            });
            search.total = 0;
            search.current = None;
            search.status = None;
        }
        set_input_value(&search_input, "", window, cx);
        self.clear_terminal_search_target();
        search_input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_terminal_search(
        &mut self,
        terminal_focus: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        {
            let mut search = self.terminal_search_mut();
            if !search.open && !search.visible {
                return;
            }
            search.open = false;
            search.visible = true;
            search.animation = Some(TerminalSearchAnimation {
                started_at: Instant::now(),
                duration: OVERLAY_ENTER_DURATION,
                from: search.visibility,
                to: 0.0,
            });
        }
        self.clear_terminal_search_target();
        self.search_target.set(None);
        window.focus(terminal_focus, cx);
        cx.notify();
    }

    pub(super) fn update_terminal_search(&mut self, pattern: String, cx: &mut Context<Self>) {
        let Some(tab_id) = self.search_target.get() else {
            return;
        };
        let Some(mut session) = self.tab_mut(tab_id) else {
            return;
        };
        let mut next_total = 0;
        let mut next_current = None;
        let mut next_status = None;
        let escaped = regex_escape(pattern.trim());
        if escaped.is_empty() {
            session.terminal.clear_search();
        } else {
            match session.terminal.set_search(&escaped) {
                Ok(total) => {
                    next_total = total;
                    next_current = if total == 0 { None } else { Some(0) };
                    next_status = if total == 0 {
                        Some(i18n::string("search.messages.no_matches"))
                    } else {
                        None
                    };
                }
                Err(error) => {
                    let error = error.to_string();
                    next_status = Some(i18n::string_args(
                        "search.messages.invalid_search",
                        &[("error", &error)],
                    ));
                }
            }
        }
        drop(session);
        {
            let mut search = self.terminal_search_mut();
            search.total = next_total;
            search.current = next_current;
            search.status = next_status;
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn terminal_search_next(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.search_target.get() else {
            return;
        };
        let Some(mut session) = self.tab_mut(tab_id) else {
            return;
        };
        session.terminal.next_match();
        drop(session);
        {
            let mut search = self.terminal_search_mut();
            if search.total > 0 {
                search.current = Some(
                    search
                        .current
                        .map_or(0, |index| (index + 1) % search.total.max(1)),
                );
            }
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn terminal_search_prev(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.search_target.get() else {
            return;
        };
        let Some(mut session) = self.tab_mut(tab_id) else {
            return;
        };
        session.terminal.prev_match();
        drop(session);
        {
            let mut search = self.terminal_search_mut();
            if search.total > 0 {
                search.current = Some(match search.current {
                    Some(0) | None => search.total - 1,
                    Some(index) => index - 1,
                });
            }
        }
        cx.notify();
    }

    fn clear_terminal_search_target(&self) {
        if let Some(tab_id) = self.search_target.get()
            && let Some(mut session) = self.tab_mut(tab_id)
        {
            session.terminal.clear_search();
        }
    }
}

fn regex_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
            | '#' | '&' | '-' | '~' => {
                out.push('\\');
                out.push(character);
            }
            _ => out.push(character),
        }
    }
    out
}
