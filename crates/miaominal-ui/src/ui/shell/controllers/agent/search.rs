use super::{
    AgentController, SessionAgentMessageRole, SessionAgentState, split_message_into_blocks,
};
use crate::ui::i18n;
use crate::ui::shell::ChatPanelView;
use crate::ui::shell::forms::TerminalSearchAnimation;
use crate::ui::shell::session_agent_view::SessionAgentConversationView;
use crate::ui::shell::{OVERLAY_ENTER_DURATION, set_input_value};
use gpui::{Context, Entity, Window, px};
use gpui_component::input::InputState;
use std::time::{Duration, Instant};

const CONVERSATION_SEARCH_STREAM_REFRESH_INTERVAL: Duration = Duration::from_millis(100);

impl AgentController {
    pub(in crate::ui::shell) fn session_filter_input(&self) -> Entity<InputState> {
        self.forms.chat_search.session_filter_input.clone()
    }

    pub(in crate::ui::shell) fn conversation_search_input(&self) -> Entity<InputState> {
        self.forms.chat_search.conversation_search_input.clone()
    }

    pub(in crate::ui::shell) fn session_filter_open(&self) -> bool {
        self.forms.chat_search.session_filter_open
    }

    pub(in crate::ui::shell) fn conversation_search_open(&self) -> bool {
        self.forms.chat_search.conversation_search_open
    }

    pub(in crate::ui::shell) fn conversation_search_match_count(&self) -> usize {
        self.forms.chat_search.match_count
    }

    pub(in crate::ui::shell) fn conversation_search_current_match(&self) -> Option<usize> {
        self.forms.chat_search.current_match
    }

    pub(in crate::ui::shell) fn conversation_search_status(&self) -> Option<String> {
        self.forms.chat_search.status.clone()
    }

    pub(in crate::ui::shell) fn open_session_filter(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forms = &mut self.forms.chat_search;
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
        let forms = &mut self.forms.chat_search;
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
        self.runtime.get_mut().foreground.search_query = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn reset_session_filter(&mut self, cx: &mut Context<Self>) {
        let forms = &mut self.forms.chat_search;
        forms.session_filter_open = false;
        forms.session_filter_visible = false;
        forms.session_filter_visibility = 0.0;
        forms.session_filter_animation = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_conversation_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forms = &mut self.forms.chat_search;
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
        let forms = &mut self.forms.chat_search;
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
        self.clear_conversation_search_runtime(cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn reset_conversation_search(&mut self, cx: &mut Context<Self>) {
        self.clear_conversation_search_runtime(cx);
        let forms = &mut self.forms.chat_search;
        forms.conversation_search_open = false;
        forms.conversation_search_visible = false;
        forms.conversation_search_visibility = 0.0;
        forms.conversation_search_animation = None;
        forms.match_count = 0;
        forms.current_match = None;
        forms.status = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn update_conversation_search(
        &mut self,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let query = query.trim().to_string();
        if query.is_empty() {
            self.clear_conversation_search_runtime(cx);
            let forms = &mut self.forms.chat_search;
            forms.match_count = 0;
            forms.current_match = None;
            forms.status = None;
            cx.notify();
            return;
        }

        let query_lower = query.to_lowercase();
        let mut match_indices = Vec::new();
        {
            let state = &self.runtime.get_mut().foreground;
            for (message_index, message) in state.messages.iter().enumerate() {
                if !matches!(
                    message.role,
                    SessionAgentMessageRole::User | SessionAgentMessageRole::Assistant
                ) {
                    continue;
                }
                for (block_index, block) in split_message_into_blocks(&message.content)
                    .iter()
                    .enumerate()
                {
                    if block.to_lowercase().contains(&query_lower) {
                        match_indices.push((message_index, block_index));
                    }
                }
            }
        }

        let count = match_indices.len();
        let conversation = {
            let state = &mut self.runtime.get_mut().foreground;
            invalidate_search_refresh(state);
            state.search_query = Some(query);
            state.search_match_indices = match_indices;
            state.search_current_match = (count > 0).then_some(0);
            if count == 0 {
                state.search_scroll_target = None;
            }
            state.conversation_view.as_ref().cloned()
        };
        if let Some(conversation) = conversation {
            conversation.read(cx).remeasure_all();
        }
        if count > 0 {
            self.request_conversation_search_scroll_to_match(0, cx);
        }

        let forms = &mut self.forms.chat_search;
        forms.match_count = count;
        forms.current_match = (count > 0).then_some(0);
        forms.status = (count == 0).then(|| i18n::string("search.messages.no_matches"));
        cx.notify();
    }

    pub(in crate::ui::shell) fn navigate_conversation_search_next(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let next = {
            let state = &mut self.runtime.get_mut().foreground;
            let count = state.search_match_indices.len();
            if count == 0 {
                return;
            }
            let next = state
                .search_current_match
                .map_or(0, |index| (index + 1) % count);
            state.search_current_match = Some(next);
            next
        };
        self.request_conversation_search_scroll_to_match(next, cx);
        self.forms.chat_search.current_match = Some(next);
        cx.notify();
    }

    pub(in crate::ui::shell) fn navigate_conversation_search_prev(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let previous = {
            let state = &mut self.runtime.get_mut().foreground;
            let count = state.search_match_indices.len();
            if count == 0 {
                return;
            }
            let previous = match state.search_current_match {
                Some(0) | None => count - 1,
                Some(index) => index - 1,
            };
            state.search_current_match = Some(previous);
            previous
        };
        self.request_conversation_search_scroll_to_match(previous, cx);
        self.forms.chat_search.current_match = Some(previous);
        cx.notify();
    }

    pub(in crate::ui::shell) fn refresh_conversation_search_message(
        &mut self,
        message_index: usize,
        cx: &mut Context<Self>,
    ) {
        let outcome = refresh_conversation_search_message_state(
            &mut self.runtime.get_mut().foreground,
            message_index,
        );
        self.apply_search_refresh_outcome(outcome, cx);
    }

    pub(in crate::ui::shell) fn schedule_conversation_search_message_refresh(
        &mut self,
        message_index: usize,
        cx: &mut Context<Self>,
    ) {
        let scheduled = {
            let state = &mut self.runtime.get_mut().foreground;
            if state
                .search_query
                .as_ref()
                .is_none_or(|query| query.trim().is_empty())
            {
                return;
            }

            let should_schedule = state.search_refresh_pending_messages.is_empty();
            if !state
                .search_refresh_pending_messages
                .contains(&message_index)
            {
                state.search_refresh_pending_messages.push(message_index);
            }
            if !should_schedule {
                return;
            }

            state.search_refresh_generation = state.search_refresh_generation.wrapping_add(1);
            (state.session_id.clone(), state.search_refresh_generation)
        };
        let (Some(session_id), generation) = scheduled else {
            self.refresh_conversation_search_message(message_index, cx);
            return;
        };

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(CONVERSATION_SEARCH_STREAM_REFRESH_INTERVAL)
                .await;
            let _ = this.update(cx, move |controller, cx| {
                controller.refresh_pending_conversation_search_messages(
                    &session_id,
                    generation,
                    cx,
                );
            });
        })
        .detach();
    }

    pub(in crate::ui::shell) fn clear_conversation_search_state(&mut self, cx: &mut Context<Self>) {
        self.clear_conversation_search_runtime(cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn clear_conversation_search_scroll_target(
        &mut self,
        target: (usize, usize),
        cx: &mut Context<Self>,
    ) {
        let state = &mut self.runtime.get_mut().foreground;
        if state.search_scroll_target == Some(target) {
            state.search_scroll_target = None;
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn advance_conversation_search_bar(
        &mut self,
        window: &mut Window,
    ) -> Option<f32> {
        let forms = &mut self.forms.chat_search;
        advance_search_bar(
            forms.conversation_search_open,
            &mut forms.conversation_search_visible,
            &mut forms.conversation_search_visibility,
            &mut forms.conversation_search_animation,
            window,
        )
    }

    pub(in crate::ui::shell) fn advance_session_filter_bar(
        &mut self,
        window: &mut Window,
    ) -> Option<f32> {
        let forms = &mut self.forms.chat_search;
        advance_search_bar(
            forms.session_filter_open,
            &mut forms.session_filter_visible,
            &mut forms.session_filter_visibility,
            &mut forms.session_filter_animation,
            window,
        )
    }

    fn refresh_pending_conversation_search_messages(
        &mut self,
        session_id: &str,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let outcomes = {
            let runtime = self.runtime.get_mut();
            let Some(state) = runtime.session_mut(session_id) else {
                return;
            };
            if state.search_refresh_generation != generation {
                return;
            }
            let pending = std::mem::take(&mut state.search_refresh_pending_messages);
            pending
                .into_iter()
                .map(|message_index| {
                    refresh_conversation_search_message_state(state, message_index)
                })
                .collect::<Vec<_>>()
        };
        for outcome in outcomes {
            self.apply_search_refresh_outcome(outcome, cx);
        }
    }

    fn apply_search_refresh_outcome(
        &mut self,
        outcome: Option<SearchRefreshOutcome>,
        cx: &mut Context<Self>,
    ) {
        let Some(outcome) = outcome else {
            return;
        };
        let forms = &mut self.forms.chat_search;
        forms.match_count = outcome.match_count;
        forms.current_match = outcome.current_match;
        forms.status =
            (outcome.match_count == 0).then(|| i18n::string("search.messages.no_matches"));
        if let Some((message_index, conversation)) = outcome.first_phase_scroll {
            conversation.read(cx).scroll_to(message_index, px(0.0));
        }
        cx.notify();
    }

    fn clear_conversation_search_runtime(&mut self, cx: &gpui::App) {
        let conversation = {
            let state = &mut self.runtime.get_mut().foreground;
            invalidate_search_refresh(state);
            state.search_query = None;
            state.search_match_indices.clear();
            state.search_current_match = None;
            state.search_scroll_target = None;
            state.conversation_view.as_ref().cloned()
        };
        if let Some(conversation) = conversation {
            conversation.read(cx).remeasure_all();
        }
    }

    fn request_conversation_search_scroll_to_match(
        &mut self,
        match_list_index: usize,
        cx: &mut Context<Self>,
    ) {
        let (target, conversation) = {
            let state = &mut self.runtime.get_mut().foreground;
            let Some(target) = state.search_match_indices.get(match_list_index).copied() else {
                return;
            };
            state.search_scroll_target = Some(target);
            let conversation = (state.panel_view == ChatPanelView::Conversation)
                .then(|| state.conversation_view.as_ref().cloned())
                .flatten();
            (target, conversation)
        };
        if let Some(conversation) = conversation {
            conversation.read(cx).scroll_to(target.0, px(0.0));
        }
    }
}

struct SearchRefreshOutcome {
    match_count: usize,
    current_match: Option<usize>,
    first_phase_scroll: Option<(usize, Entity<SessionAgentConversationView>)>,
}

fn refresh_conversation_search_message_state(
    state: &mut SessionAgentState,
    message_index: usize,
) -> Option<SearchRefreshOutcome> {
    state
        .search_refresh_pending_messages
        .retain(|index| *index != message_index);
    let query = state
        .search_query
        .as_ref()
        .map(|query| query.trim().to_lowercase())
        .filter(|query| !query.is_empty())?;

    let previous_match_count = state.search_match_indices.len();
    let previous_position = state.search_current_match.unwrap_or(0);
    let previous_target = state
        .search_current_match
        .and_then(|index| state.search_match_indices.get(index).copied());
    state
        .search_match_indices
        .retain(|(index, _)| *index != message_index);

    let new_matches = state
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
    let insert_at = state
        .search_match_indices
        .partition_point(|(index, _)| *index < message_index);
    state
        .search_match_indices
        .splice(insert_at..insert_at, new_matches);

    let match_count = state.search_match_indices.len();
    let current_match = resolve_conversation_search_current_match(
        &state.search_match_indices,
        previous_target,
        previous_position,
    );
    state.search_current_match = current_match;
    if state
        .search_scroll_target
        .is_some_and(|target| !state.search_match_indices.contains(&target))
    {
        state.search_scroll_target = None;
    }
    let should_reveal_current = current_match.is_some()
        && (previous_match_count == 0
            || previous_target.is_some_and(|target| !state.search_match_indices.contains(&target)));
    let first_phase_scroll = if should_reveal_current {
        current_match
            .and_then(|index| state.search_match_indices.get(index).copied())
            .and_then(|target| {
                state.search_scroll_target = Some(target);
                (state.panel_view == ChatPanelView::Conversation)
                    .then(|| state.conversation_view.as_ref().cloned())
                    .flatten()
                    .map(|conversation| (target.0, conversation))
            })
    } else {
        None
    };
    Some(SearchRefreshOutcome {
        match_count,
        current_match,
        first_phase_scroll,
    })
}

fn invalidate_search_refresh(state: &mut SessionAgentState) {
    state.search_refresh_generation = state.search_refresh_generation.wrapping_add(1);
    state.search_refresh_pending_messages.clear();
}

fn advance_search_bar(
    open: bool,
    visible: &mut bool,
    visibility: &mut f32,
    animation: &mut Option<TerminalSearchAnimation>,
    window: &mut Window,
) -> Option<f32> {
    if let Some(active_animation) = *animation {
        let duration_seconds = active_animation.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            *visibility = active_animation.to;
            *animation = None;
        } else {
            let elapsed = Instant::now().saturating_duration_since(active_animation.started_at);
            let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
            let eased = progress * progress * (3.0 - 2.0 * progress);
            *visibility =
                active_animation.from + (active_animation.to - active_animation.from) * eased;
            if progress >= 1.0 {
                *visibility = active_animation.to;
                *animation = None;
            } else {
                window.request_animation_frame();
            }
        }
    }

    if *visibility <= f32::EPSILON && !open {
        *visible = false;
        return None;
    }
    if open || *visibility > f32::EPSILON {
        *visible = true;
        return Some((*visibility).clamp(0.0, 1.0));
    }
    *visible = false;
    None
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
