use super::super::forms::TerminalSearchAnimation;
use super::super::*;
use crate::ui::i18n;

const CONVERSATION_SEARCH_STREAM_REFRESH_INTERVAL: Duration = Duration::from_millis(100);

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
        forms.session_filter_visible = true;
        forms.session_filter_animation = Some(TerminalSearchAnimation {
            started_at: Instant::now(),
            duration: OVERLAY_ENTER_DURATION,
            from: forms.session_filter_visibility,
            to: 1.0,
        });
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
        forms.session_filter_visible = true;
        forms.session_filter_animation = Some(TerminalSearchAnimation {
            started_at: Instant::now(),
            duration: OVERLAY_ENTER_DURATION,
            from: forms.session_filter_visibility,
            to: 0.0,
        });
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
        self.clear_conversation_search_state(cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn update_conversation_search(
        &mut self,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let query = query.trim().to_string();
        if query.is_empty() {
            self.clear_conversation_search_state(cx);
            let forms = &mut self.workspace_forms.chat_search;
            forms.match_count = 0;
            forms.current_match = None;
            forms.status = None;
            cx.notify();
            return;
        }

        self.cancel_conversation_search_refresh();
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
        if let Some(conversation) = self.session_agent.conversation_view.as_ref() {
            conversation.read(cx).remeasure_all();
        }
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

    pub(in crate::ui::shell) fn refresh_conversation_search_message(
        &mut self,
        message_index: usize,
        cx: &mut Context<Self>,
    ) {
        self.session_agent
            .search_refresh_pending_messages
            .retain(|index| *index != message_index);
        let Some(query) = self
            .session_agent
            .search_query
            .as_ref()
            .map(|query| query.trim().to_lowercase())
            .filter(|query| !query.is_empty())
        else {
            return;
        };

        let previous_match_count = self.session_agent.search_match_indices.len();
        let previous_position = self.session_agent.search_current_match.unwrap_or(0);
        let previous_target = self
            .session_agent
            .search_current_match
            .and_then(|index| self.session_agent.search_match_indices.get(index).copied());
        self.session_agent
            .search_match_indices
            .retain(|(index, _)| *index != message_index);

        let new_matches = self
            .session_agent
            .messages
            .get(message_index)
            .filter(|message| {
                matches!(
                    message.role,
                    SessionAgentMessageRole::User | SessionAgentMessageRole::Assistant
                )
            })
            .map(|message| {
                split_message_into_blocks(&message.content)
                    .into_iter()
                    .enumerate()
                    .filter_map(|(block_index, block)| {
                        block
                            .to_lowercase()
                            .contains(&query)
                            .then_some((message_index, block_index))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let insert_at = self
            .session_agent
            .search_match_indices
            .partition_point(|(index, _)| *index < message_index);
        self.session_agent
            .search_match_indices
            .splice(insert_at..insert_at, new_matches);

        let match_count = self.session_agent.search_match_indices.len();
        let current_match = resolve_conversation_search_current_match(
            &self.session_agent.search_match_indices,
            previous_target,
            previous_position,
        );
        self.session_agent.search_current_match = current_match;
        let previous_target_was_removed = self
            .session_agent
            .search_scroll_target
            .is_some_and(|target| !self.session_agent.search_match_indices.contains(&target));
        if previous_target_was_removed {
            self.session_agent.search_scroll_target = None;
        }
        let should_reveal_current = current_match.is_some()
            && (previous_match_count == 0
                || previous_target.is_some_and(|target| {
                    !self.session_agent.search_match_indices.contains(&target)
                }));

        let forms = &mut self.workspace_forms.chat_search;
        forms.match_count = match_count;
        forms.current_match = current_match;
        forms.status = (match_count == 0).then(|| i18n::string("search.messages.no_matches"));
        if should_reveal_current && let Some(current_match) = current_match {
            self.request_conversation_search_scroll_to_match(current_match, cx);
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn schedule_conversation_search_message_refresh(
        &mut self,
        message_index: usize,
        cx: &mut Context<Self>,
    ) {
        if self
            .session_agent
            .search_query
            .as_ref()
            .is_none_or(|query| query.trim().is_empty())
        {
            return;
        }

        let should_schedule = self
            .session_agent
            .search_refresh_pending_messages
            .is_empty();
        if !self
            .session_agent
            .search_refresh_pending_messages
            .contains(&message_index)
        {
            self.session_agent
                .search_refresh_pending_messages
                .push(message_index);
        }
        if !should_schedule {
            return;
        }

        let Some(session_id) = self.session_agent.session_id.clone() else {
            self.refresh_conversation_search_message(message_index, cx);
            return;
        };
        self.session_agent.search_refresh_generation =
            self.session_agent.search_refresh_generation.wrapping_add(1);
        let generation = self.session_agent.search_refresh_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(CONVERSATION_SEARCH_STREAM_REFRESH_INTERVAL)
                .await;
            let _ = this.update(cx, move |this, cx| {
                this.with_session_agent_state(&session_id, |this| {
                    if this.session_agent.search_refresh_generation != generation {
                        return;
                    }
                    let pending =
                        std::mem::take(&mut this.session_agent.search_refresh_pending_messages);
                    for message_index in pending {
                        this.refresh_conversation_search_message(message_index, cx);
                    }
                });
            });
        })
        .detach();
    }

    pub(in crate::ui::shell) fn cancel_conversation_search_refresh(&mut self) {
        self.session_agent.search_refresh_generation =
            self.session_agent.search_refresh_generation.wrapping_add(1);
        self.session_agent.search_refresh_pending_messages.clear();
    }

    pub(in crate::ui::shell) fn clear_conversation_search_state(&mut self, cx: &mut Context<Self>) {
        self.cancel_conversation_search_refresh();
        self.session_agent.search_query = None;
        self.session_agent.search_match_indices.clear();
        self.session_agent.search_current_match = None;
        self.session_agent.search_scroll_target = None;
        if let Some(conversation) = self.session_agent.conversation_view.as_ref() {
            conversation.read(cx).remeasure_all();
        }
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
        if !self.panels.session_agent_panel_open
            || self.active_terminal_session_index().is_none()
            || self.session_agent.panel_view != ChatPanelView::Conversation
            || self
                .workspace_state
                .session_agent_text_drag_conversation
                .is_some()
        {
            return;
        }
        let conversation = self.ensure_session_agent_conversation_view(cx);
        conversation.read(cx).scroll_to(target.0, px(0.0));
    }
}

fn resolve_conversation_search_current_match(
    matches: &[(usize, usize)],
    previous_target: Option<(usize, usize)>,
    previous_position: usize,
) -> Option<usize> {
    previous_target
        .and_then(|target| matches.iter().position(|candidate| *candidate == target))
        .or_else(|| {
            matches
                .len()
                .checked_sub(1)
                .map(|last_index| previous_position.min(last_index))
        })
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

#[cfg(test)]
mod tests {
    use super::resolve_conversation_search_current_match;

    #[test]
    fn zero_conversation_search_matches_have_no_current_index() {
        assert_eq!(
            resolve_conversation_search_current_match(&[], Some((4, 2)), 3),
            None
        );
    }

    #[test]
    fn conversation_search_refresh_preserves_or_clamps_the_current_match() {
        let matches = [(1, 0), (4, 2)];

        assert_eq!(
            resolve_conversation_search_current_match(&matches, Some((4, 2)), 0),
            Some(1)
        );
        assert_eq!(
            resolve_conversation_search_current_match(&matches, Some((9, 9)), 8),
            Some(1)
        );
    }
}
