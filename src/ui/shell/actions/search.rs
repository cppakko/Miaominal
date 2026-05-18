use super::super::forms::TerminalSearchAnimation;
use super::super::*;
use crate::ui::i18n;

impl AppView {
    pub(in crate::ui::shell) fn open_terminal_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .is_some_and(|tab| tab.as_session().is_some())
        {
            return;
        }
        let search_input = self.workspace_forms.search.input.clone();
        {
            let search = &mut self.workspace_forms.search;
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
        self.clear_active_terminal_search();
        search_input.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_terminal_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workspace_forms.search.open && !self.workspace_forms.search.visible {
            return;
        }
        let search = &mut self.workspace_forms.search;
        search.open = false;
        search.visible = true;
        search.animation = Some(TerminalSearchAnimation {
            started_at: Instant::now(),
            duration: OVERLAY_ENTER_DURATION,
            from: search.visibility,
            to: 0.0,
        });
        self.clear_active_terminal_search();
        window.focus(
            &self.workspace_state.workspace.active_pane.terminal_focus,
            cx,
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn update_terminal_search(
        &mut self,
        pattern: String,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.workspace_state.workspace.active_tab else {
            return;
        };
        let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
            return;
        };
        let Some(session) = tab.as_session_mut() else {
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
        self.workspace_forms.search.total = next_total;
        self.workspace_forms.search.current = next_current;
        self.workspace_forms.search.status = next_status;
        cx.notify();
    }

    pub(in crate::ui::shell) fn terminal_search_next(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.workspace_state.workspace.active_tab else {
            return;
        };
        let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
            return;
        };
        let Some(session) = tab.as_session_mut() else {
            return;
        };
        session.terminal.next_match();
        let search = &mut self.workspace_forms.search;
        if search.total > 0 {
            search.current = Some(
                search
                    .current
                    .map_or(0, |idx| (idx + 1) % search.total.max(1)),
            );
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn terminal_search_prev(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.workspace_state.workspace.active_tab else {
            return;
        };
        let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
            return;
        };
        let Some(session) = tab.as_session_mut() else {
            return;
        };
        session.terminal.prev_match();
        let search = &mut self.workspace_forms.search;
        if search.total > 0 {
            search.current = Some(match search.current {
                Some(0) | None => search.total - 1,
                Some(idx) => idx - 1,
            });
        }
        cx.notify();
    }

    fn clear_active_terminal_search(&mut self) {
        if let Some(index) = self.workspace_state.workspace.active_tab
            && let Some(tab) = self.workspace_state.tabs.get_mut(index)
            && let Some(session) = tab.as_session_mut()
        {
            session.terminal.clear_search();
        }
    }
}

/// Escape every regex metacharacter so that the user-entered pattern is treated
/// as a literal substring (safer default than full regex). Mirrors what
/// `regex::escape` does without pulling in a new dependency.
fn regex_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
            | '#' | '&' | '-' | '~' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}
