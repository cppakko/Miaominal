use super::super::forms::TerminalSearchAnimation;
use super::super::*;
use crate::ui::i18n;

impl AppView {
    pub(in crate::ui::shell) fn open_terminal_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .is_none_or(|tab| tab.as_session().is_none())
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

    // ── Chat search actions ──

    pub(in crate::ui::shell) fn open_session_filter(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forms = &mut self.workspace_forms.chat_search;
        if forms.session_filter_open {
            forms
                .session_filter_input
                .update(cx, |state, cx| state.focus(window, cx));
            return;
        }
        forms.session_filter_open = true;
        set_input_value(&forms.session_filter_input, "", window, cx);
        forms
            .session_filter_input
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_session_filter(&mut self, cx: &mut Context<Self>) {
        let forms = &mut self.workspace_forms.chat_search;
        if !forms.session_filter_open {
            return;
        }
        forms.session_filter_open = false;
        self.session_agent.search_query = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_conversation_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forms = &mut self.workspace_forms.chat_search;
        if forms.conversation_search_open {
            forms
                .conversation_search_input
                .update(cx, |state, cx| state.focus(window, cx));
            return;
        }
        forms.conversation_search_open = true;
        forms.conversation_search_visible = true;
        forms.conversation_search_animation = Some(TerminalSearchAnimation {
            started_at: Instant::now(),
            duration: OVERLAY_ENTER_DURATION,
            from: forms.conversation_search_visibility,
            to: 1.0,
        });
        forms.match_count = 0;
        forms.current_match = None;
        forms.status = None;
        set_input_value(&forms.conversation_search_input, "", window, cx);
        forms
            .conversation_search_input
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_conversation_search(&mut self, cx: &mut Context<Self>) {
        let forms = &mut self.workspace_forms.chat_search;
        if !forms.conversation_search_open && !forms.conversation_search_visible {
            return;
        }
        forms.conversation_search_open = false;
        forms.conversation_search_visible = true;
        forms.conversation_search_animation = Some(TerminalSearchAnimation {
            started_at: Instant::now(),
            duration: OVERLAY_ENTER_DURATION,
            from: forms.conversation_search_visibility,
            to: 0.0,
        });
        self.session_agent.search_query = None;
        self.session_agent.search_match_indices.clear();
        self.session_agent.search_current_match = None;
        self.session_agent.search_scroll_target = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn update_conversation_search(
        &mut self,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let query = query.trim().to_string();
        if query.is_empty() {
            self.session_agent.search_query = None;
            self.session_agent.search_match_indices.clear();
            self.session_agent.search_current_match = None;
            self.session_agent.search_scroll_target = None;
            let forms = &mut self.workspace_forms.chat_search;
            forms.match_count = 0;
            forms.current_match = None;
            forms.status = None;
            cx.notify();
            return;
        }

        let query_lower = query.to_lowercase();
        let mut match_indices: Vec<(usize, usize)> = Vec::new();

        for (msg_idx, message) in self.session_agent.messages.iter().enumerate() {
            if !matches!(
                message.role,
                SessionAgentMessageRole::User | SessionAgentMessageRole::Assistant
            ) {
                continue;
            }
            let blocks = split_message_into_blocks(&message.content);
            for (block_idx, block) in blocks.iter().enumerate() {
                if block.to_lowercase().contains(&query_lower) {
                    match_indices.push((msg_idx, block_idx));
                }
            }
        }

        let count = match_indices.len();
        self.session_agent.search_query = Some(query);
        self.session_agent.search_match_indices = match_indices;
        self.session_agent.search_current_match = if count > 0 { Some(0) } else { None };
        if count > 0 {
            self.request_conversation_search_scroll_to_match(0, cx);
        } else {
            self.session_agent.search_scroll_target = None;
        }

        let forms = &mut self.workspace_forms.chat_search;
        forms.match_count = count;
        forms.current_match = if count > 0 { Some(0) } else { None };
        forms.status = if count == 0 {
            Some(i18n::string("search.messages.no_matches"))
        } else {
            None
        };

        cx.notify();
    }

    pub(in crate::ui::shell) fn navigate_conversation_search_next(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let count = self.session_agent.search_match_indices.len();
        if count == 0 {
            return;
        }
        let next = self
            .session_agent
            .search_current_match
            .map_or(0, |idx| (idx + 1) % count);
        self.session_agent.search_current_match = Some(next);
        self.request_conversation_search_scroll_to_match(next, cx);
        self.workspace_forms.chat_search.current_match = Some(next);
        cx.notify();
    }

    pub(in crate::ui::shell) fn navigate_conversation_search_prev(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let count = self.session_agent.search_match_indices.len();
        if count == 0 {
            return;
        }
        let prev = match self.session_agent.search_current_match {
            Some(0) | None => count - 1,
            Some(idx) => idx - 1,
        };
        self.session_agent.search_current_match = Some(prev);
        self.request_conversation_search_scroll_to_match(prev, cx);
        self.workspace_forms.chat_search.current_match = Some(prev);
        cx.notify();
    }

    fn request_conversation_search_scroll_to_match(
        &mut self,
        match_list_index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self
            .session_agent
            .search_match_indices
            .get(match_list_index)
            .copied()
        else {
            return;
        };

        self.session_agent.search_scroll_target = Some(target);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(96))
                .await;
            let _ = this.update(cx, move |this, cx| {
                if this.session_agent.search_scroll_target == Some(target) {
                    this.session_agent.search_scroll_target = None;
                    cx.notify();
                }
            });
        })
        .detach();
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
