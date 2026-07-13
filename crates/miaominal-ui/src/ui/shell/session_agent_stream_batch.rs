use miaominal_agent::AgentChatEvent;
use std::time::Duration;

/// Maximum amount of time stream deltas should wait before being applied to the UI.
pub(in crate::ui::shell) const SESSION_AGENT_STREAM_UI_FLUSH_INTERVAL: Duration =
    Duration::from_millis(16);

/// A pending group of agent events destined for one UI update.
///
/// Only adjacent deltas with the same destination are coalesced. All other events remain in
/// arrival order so applying a batch is equivalent to applying the original event stream.
#[derive(Debug, Default)]
pub(in crate::ui::shell) struct SessionAgentStreamBatch {
    events: Vec<AgentChatEvent>,
}

impl SessionAgentStreamBatch {
    pub(in crate::ui::shell) fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(in crate::ui::shell) fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    #[cfg(test)]
    pub(in crate::ui::shell) fn len(&self) -> usize {
        self.events.len()
    }

    /// Adds an event, appending its suffix to the previous event when both events target the
    /// same streamed field.
    pub(in crate::ui::shell) fn push(&mut self, event: AgentChatEvent) {
        match (self.events.last_mut(), event) {
            (Some(AgentChatEvent::TextDelta(current)), AgentChatEvent::TextDelta(suffix))
                if !current.trim().is_empty() =>
            {
                // A leading whitespace-only TextDelta is ignored when no assistant row exists,
                // but is meaningful when a row already exists. Without model state here, keep
                // that boundary intact. Once the accumulated delta contains visible text, later
                // suffixes are safe to merge: applying the visible prefix creates/targets the row.
                current.push_str(&suffix)
            }
            (
                Some(AgentChatEvent::ThinkingDelta(current)),
                AgentChatEvent::ThinkingDelta(suffix),
            ) if !current.trim().is_empty() && !suffix.trim().is_empty() => {
                current.push_str(&suffix)
            }
            (
                Some(AgentChatEvent::ToolCallDelta {
                    id: current_id,
                    delta: current,
                }),
                AgentChatEvent::ToolCallDelta { id, delta: suffix },
            ) if *current_id == id && !current.trim().is_empty() && !suffix.trim().is_empty() => {
                current.push_str(&suffix)
            }
            (_, event) => self.events.push(event),
        }
    }

    /// Removes and returns all pending events.
    pub(in crate::ui::shell) fn take(&mut self) -> Vec<AgentChatEvent> {
        std::mem::take(&mut self.events)
    }

    /// Applies all pending events in order and returns the number of events applied.
    #[cfg(test)]
    pub(in crate::ui::shell) fn flush(&mut self, mut apply: impl FnMut(AgentChatEvent)) -> usize {
        let event_count = self.events.len();
        for event in self.events.drain(..) {
            apply(event);
        }
        event_count
    }
}

/// Returns whether an event should bypass the 16 ms timer and flush the pending batch now.
///
/// Streaming content and token accounting may be delayed until the next UI tick. Tool lifecycle
/// and terminal events can change control flow, so they must be observed synchronously and in
/// order with any deltas already pending.
pub(in crate::ui::shell) fn session_agent_event_requires_immediate_flush(
    event: &AgentChatEvent,
) -> bool {
    !matches!(
        event,
        AgentChatEvent::TextDelta(_)
            | AgentChatEvent::ThinkingDelta(_)
            | AgentChatEvent::ToolCallDelta { .. }
            | AgentChatEvent::TokenUsage { .. }
    )
}

pub(in crate::ui::shell) fn session_agent_event_is_finished(event: &AgentChatEvent) -> bool {
    matches!(event, AgentChatEvent::Finished(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_agent::AgentChatToolEvent;

    #[test]
    fn flush_interval_is_one_ui_frame() {
        assert_eq!(
            SESSION_AGENT_STREAM_UI_FLUSH_INTERVAL,
            Duration::from_millis(16)
        );
    }

    #[test]
    fn coalesces_adjacent_unicode_text_and_thinking_deltas() {
        let mut batch = SessionAgentStreamBatch::new();

        batch.push(AgentChatEvent::TextDelta("你".into()));
        batch.push(AgentChatEvent::TextDelta("好，🐱".into()));
        batch.push(AgentChatEvent::ThinkingDelta("推".into()));
        batch.push(AgentChatEvent::ThinkingDelta("理🧠".into()));

        assert_eq!(
            batch.take(),
            vec![
                AgentChatEvent::TextDelta("你好，🐱".into()),
                AgentChatEvent::ThinkingDelta("推理🧠".into()),
            ]
        );
    }

    #[test]
    fn coalesces_only_adjacent_tool_deltas_with_the_same_id() {
        let mut batch = SessionAgentStreamBatch::new();

        batch.push(tool_delta("first", "{"));
        batch.push(tool_delta("first", "\"path\":"));
        batch.push(tool_delta("second", "other"));
        batch.push(tool_delta("first", "\"a\"}"));

        assert_eq!(
            batch.take(),
            vec![
                tool_delta("first", "{\"path\":"),
                tool_delta("second", "other"),
                tool_delta("first", "\"a\"}"),
            ]
        );
    }

    #[test]
    fn does_not_merge_deltas_separated_by_another_event() {
        let mut batch = SessionAgentStreamBatch::new();

        batch.push(AgentChatEvent::TextDelta("before".into()));
        batch.push(AgentChatEvent::ThinkingDelta("reasoning".into()));
        batch.push(AgentChatEvent::TextDelta("after".into()));

        assert_eq!(
            batch.take(),
            vec![
                AgentChatEvent::TextDelta("before".into()),
                AgentChatEvent::ThinkingDelta("reasoning".into()),
                AgentChatEvent::TextDelta("after".into()),
            ]
        );
    }

    #[test]
    fn preserves_leading_whitespace_text_delta_boundary() {
        let mut batch = SessionAgentStreamBatch::new();

        batch.push(AgentChatEvent::TextDelta("\n".into()));
        batch.push(AgentChatEvent::TextDelta("answer".into()));
        batch.push(AgentChatEvent::TextDelta("\nmore".into()));

        assert_eq!(
            batch.take(),
            vec![
                AgentChatEvent::TextDelta("\n".into()),
                AgentChatEvent::TextDelta("answer\nmore".into()),
            ]
        );
    }

    #[test]
    fn preserves_ignored_whitespace_thinking_and_tool_deltas() {
        let mut batch = SessionAgentStreamBatch::new();

        batch.push(AgentChatEvent::ThinkingDelta("reason".into()));
        batch.push(AgentChatEvent::ThinkingDelta(" \n".into()));
        batch.push(AgentChatEvent::ThinkingDelta("next".into()));
        batch.push(tool_delta("tool-1", "{"));
        batch.push(tool_delta("tool-1", "  "));
        batch.push(tool_delta("tool-1", "}"));

        assert_eq!(
            batch.take(),
            vec![
                AgentChatEvent::ThinkingDelta("reason".into()),
                AgentChatEvent::ThinkingDelta(" \n".into()),
                AgentChatEvent::ThinkingDelta("next".into()),
                tool_delta("tool-1", "{"),
                tool_delta("tool-1", "  "),
                tool_delta("tool-1", "}"),
            ]
        );
    }

    #[test]
    fn preserves_structural_event_order() {
        let started = AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
            id: "tool-1".into(),
            name: "read".into(),
            arguments: String::new(),
        });
        let completed = AgentChatEvent::ToolCallCompleted {
            id: "tool-1".into(),
            result: "ok".into(),
        };
        let finished = AgentChatEvent::Finished("answer".into());
        let mut batch = SessionAgentStreamBatch::new();

        batch.push(AgentChatEvent::TextDelta("answer".into()));
        batch.push(started.clone());
        batch.push(tool_delta("tool-1", "args"));
        batch.push(completed.clone());
        batch.push(finished.clone());

        assert_eq!(
            batch.take(),
            vec![
                AgentChatEvent::TextDelta("answer".into()),
                started,
                tool_delta("tool-1", "args"),
                completed,
                finished,
            ]
        );
    }

    #[test]
    fn take_clears_the_batch_for_reuse() {
        let mut batch = SessionAgentStreamBatch::new();
        batch.push(AgentChatEvent::TextDelta("first".into()));

        assert_eq!(
            batch.take(),
            vec![AgentChatEvent::TextDelta("first".into())]
        );
        assert!(batch.is_empty());

        batch.push(AgentChatEvent::TextDelta("second".into()));
        assert_eq!(batch.len(), 1);
    }

    #[test]
    fn flush_applies_events_in_order_and_clears_the_batch() {
        let mut batch = SessionAgentStreamBatch::new();
        batch.push(AgentChatEvent::TextDelta("a".into()));
        batch.push(AgentChatEvent::TextDelta("b".into()));
        batch.push(AgentChatEvent::Finished("ab".into()));
        let mut applied = Vec::new();

        assert_eq!(batch.flush(|event| applied.push(event)), 2);
        assert_eq!(
            applied,
            vec![
                AgentChatEvent::TextDelta("ab".into()),
                AgentChatEvent::Finished("ab".into()),
            ]
        );
        assert!(batch.is_empty());
    }

    #[test]
    fn identifies_immediate_and_finished_events() {
        let text = AgentChatEvent::TextDelta("delta".into());
        let tool_delta = tool_delta("tool-1", "delta");
        let usage = AgentChatEvent::TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
        };
        let started = AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
            id: "tool-1".into(),
            name: "read".into(),
            arguments: String::new(),
        });
        let finished = AgentChatEvent::Finished("done".into());

        assert!(!session_agent_event_requires_immediate_flush(&text));
        assert!(!session_agent_event_requires_immediate_flush(&tool_delta));
        assert!(!session_agent_event_requires_immediate_flush(&usage));
        assert!(session_agent_event_requires_immediate_flush(&started));
        assert!(session_agent_event_requires_immediate_flush(&finished));
        assert!(!session_agent_event_is_finished(&text));
        assert!(session_agent_event_is_finished(&finished));
    }

    fn tool_delta(id: &str, delta: &str) -> AgentChatEvent {
        AgentChatEvent::ToolCallDelta {
            id: id.into(),
            delta: delta.into(),
        }
    }
}
