use std::time::Instant;

use gpui_kit::{Context, FocusHandle, Window};

use super::SessionController;
use crate::ui::components::animation::{OVERLAY_ENTER_DURATION, TerminalSearchAnimation};
use crate::ui::contracts::workspace::TabId;
use crate::ui::i18n;

impl SessionController {
    pub(crate) fn sync_terminal_search_target(&self, tab_id: Option<TabId>, cx: &gpui_kit::App) {
        let Some(forms) = self.forms.as_ref() else {
            return;
        };
        if !forms.borrow().search.open {
            return;
        }
        let tab_id = tab_id.filter(|id| self.tab(*id).is_some());
        if self.search_target.get() == tab_id {
            return;
        }
        self.clear_terminal_search_target();
        self.search_target.set(tab_id);
        let pattern = forms.borrow().search.input.read(cx).value().to_string();
        self.apply_terminal_search(&pattern);
    }

    pub(in crate::ui) fn open_terminal_search(
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
        let pattern = search_input.read(cx).value().to_string();
        self.apply_terminal_search(&pattern);
        search_input.update(cx, |state, cx| state.focus(window, cx));
        if !pattern.is_empty() {
            window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
        }
        cx.notify();
    }

    pub(in crate::ui) fn close_terminal_search(
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
        self.apply_terminal_search(&pattern);
        cx.notify();
    }

    pub(in crate::ui) fn toggle_terminal_search_case_sensitive(&mut self, cx: &mut Context<Self>) {
        let pattern = {
            let mut search = self.terminal_search_mut();
            search.case_sensitive = !search.case_sensitive;
            search.input.read(cx).value().to_string()
        };
        self.apply_terminal_search(&pattern);
        cx.notify();
    }

    pub(in crate::ui) fn toggle_terminal_search_regex(&mut self, cx: &mut Context<Self>) {
        let pattern = {
            let mut search = self.terminal_search_mut();
            search.use_regex = !search.use_regex;
            search.input.read(cx).value().to_string()
        };
        self.apply_terminal_search(&pattern);
        cx.notify();
    }

    fn apply_terminal_search(&self, pattern: &str) {
        let (case_sensitive, use_regex) = {
            let mut search = self.terminal_search_mut();
            search.total = 0;
            search.current = None;
            search.status = None;
            (search.case_sensitive, search.use_regex)
        };
        let Some(tab_id) = self.search_target.get() else {
            return;
        };
        let Some(mut session) = self.tab_mut(tab_id) else {
            return;
        };
        let mut next_total = 0;
        let mut next_current = None;
        let mut next_status = None;
        let compiled = build_terminal_search_pattern(pattern, case_sensitive, use_regex);
        if compiled.is_empty() {
            session.terminal.clear_search();
        } else {
            match session.terminal.set_search(&compiled) {
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
    }

    pub(in crate::ui) fn terminal_search_next(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::ui) fn terminal_search_prev(&mut self, cx: &mut Context<Self>) {
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

fn build_terminal_search_pattern(pattern: &str, case_sensitive: bool, use_regex: bool) -> String {
    if pattern.is_empty() {
        return String::new();
    }
    let body = if use_regex {
        pattern.to_string()
    } else {
        regex_escape(pattern)
    };
    if case_sensitive {
        format!("(?-i){body}")
    } else {
        format!("(?i){body}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_search_preserves_leading_and_trailing_whitespace() {
        assert_eq!(
            build_terminal_search_pattern("  error ", false, false),
            "(?i)  error "
        );
        assert_eq!(
            build_terminal_search_pattern("[warn] ", true, false),
            r"(?-i)\[warn\] "
        );
        assert_eq!(
            build_terminal_search_pattern(r"err\d+", false, true),
            r"(?i)err\d+"
        );
        assert!(build_terminal_search_pattern("", false, false).is_empty());
    }
}
