use crate::ui::contracts::workspace::{PaneId, TabId};
use gpui_kit::{Context, Task};
use miaominal_terminal::TerminalCursorState;
use std::time::Duration;

pub(in crate::ui) const TERMINAL_CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct TerminalCursorBlinkTarget {
    pub(in crate::ui) pane_id: PaneId,
    pub(in crate::ui) tab_id: TabId,
    pub(in crate::ui) cursor: TerminalCursorState,
}

/// Manages periodic cursor visibility toggling for the currently focused terminal pane.
pub(in crate::ui) struct TerminalCursorBlink {
    visible: bool,
    target: Option<TerminalCursorBlinkTarget>,
    epoch: u64,
    _blink_task: Option<Task<()>>,
}

impl TerminalCursorBlink {
    pub(in crate::ui) fn new() -> Self {
        Self {
            visible: true,
            target: None,
            epoch: 0,
            _blink_task: None,
        }
    }

    pub(in crate::ui) fn is_visible(&self) -> bool {
        self.visible
    }

    #[cfg(test)]
    pub(in crate::ui) fn is_running(&self) -> bool {
        self._blink_task.is_some()
    }

    /// Aligns the blink timer with the currently focused viewport cursor.
    ///
    /// Keeping the same target preserves the existing blink cadence across
    /// unrelated redraws; moving the cursor or switching tabs restarts from
    /// the visible phase, and blurring or disabling blink stops with a steady
    /// visible cursor.
    pub(in crate::ui) fn sync(
        &mut self,
        next: Option<TerminalCursorBlinkTarget>,
        cx: &mut Context<Self>,
    ) {
        let next_active = next
            .filter(|candidate| candidate.cursor.blinking && candidate.cursor.visible_in_viewport);
        if self.target == next_active {
            return;
        }

        self.target = next_active;
        if next_active.is_some() {
            let was_visible = self.visible;
            self.visible = true;
            self.start_loop(cx);
            if !was_visible {
                cx.notify();
            }
        } else {
            self.stop(cx);
        }
    }

    /// Resets the cursor to visible and postpones the next fade-out after user input.
    pub(in crate::ui) fn pause_for(
        &mut self,
        pane_id: PaneId,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.target else {
            return;
        };
        if target.pane_id != pane_id || target.tab_id != tab_id {
            return;
        }

        let was_visible = self.visible;
        self.visible = true;
        self.start_loop(cx);
        if !was_visible {
            cx.notify();
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        let was_visible = self.visible;
        self.epoch = self.epoch.wrapping_add(1);
        self._blink_task = None;
        self.visible = true;
        if !was_visible {
            cx.notify();
        }
    }

    fn start_loop(&mut self, cx: &mut Context<Self>) {
        self.epoch = self.epoch.wrapping_add(1);
        let epoch = self.epoch;
        self._blink_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(TERMINAL_CURSOR_BLINK_INTERVAL)
                    .await;
                let keep_running = this
                    .update(cx, |this, cx| {
                        if this.epoch != epoch || this.target.is_none() {
                            return false;
                        }
                        this.visible = !this.visible;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext as _, TestAppContext};
    use miaominal_terminal::{TerminalCursorPosition, TerminalCursorShape};

    fn sample_target(pane_id: PaneId, tab_id: TabId, column: usize) -> TerminalCursorBlinkTarget {
        TerminalCursorBlinkTarget {
            pane_id,
            tab_id,
            cursor: TerminalCursorState {
                position: TerminalCursorPosition {
                    viewport_line: 0,
                    column,
                },
                shape: TerminalCursorShape::Block,
                blinking: true,
                visible_in_viewport: true,
            },
        }
    }

    #[gpui_kit::test]
    fn sync_and_pause_drive_terminal_cursor_blink_cadence(cx: &mut TestAppContext) {
        let blink = cx.new(|_| TerminalCursorBlink::new());

        blink.read_with(cx, |state, _| {
            assert!(state.is_visible());
            assert!(!state.is_running());
        });

        let target = sample_target(PaneId(1), TabId::new(10), 2);
        blink.update(cx, |state, cx| {
            state.sync(Some(target), cx);
        });
        blink.read_with(cx, |state, _| {
            assert!(state.is_visible());
            assert!(state.is_running());
        });

        cx.executor().advance_clock(TERMINAL_CURSOR_BLINK_INTERVAL);
        cx.executor().run_until_parked();
        blink.read_with(cx, |state, _| assert!(!state.is_visible()));

        // Re-syncing the exact same target leaves the blink cadence undisturbed.
        blink.update(cx, |state, cx| {
            state.sync(Some(target), cx);
        });
        blink.read_with(cx, |state, _| assert!(!state.is_visible()));

        // Pausing immediately restores visibility and restarts a full 500 ms interval.
        blink.update(cx, |state, cx| {
            state.pause_for(PaneId(1), TabId::new(10), cx);
        });
        blink.read_with(cx, |state, _| assert!(state.is_visible()));

        cx.executor().advance_clock(Duration::from_millis(250));
        cx.executor().run_until_parked();
        blink.read_with(cx, |state, _| assert!(state.is_visible()));

        cx.executor().advance_clock(Duration::from_millis(250));
        cx.executor().run_until_parked();
        blink.read_with(cx, |state, _| assert!(!state.is_visible()));

        // Cursor movement resets to visible cleanly.
        blink.update(cx, |state, cx| {
            state.sync(Some(sample_target(PaneId(1), TabId::new(10), 3)), cx);
        });
        blink.read_with(cx, |state, _| {
            assert!(state.is_visible());
            assert!(state.is_running());
        });

        // Disabling blinking or losing focus stops the timer with a steady visible cursor.
        cx.executor().advance_clock(TERMINAL_CURSOR_BLINK_INTERVAL);
        cx.executor().run_until_parked();
        blink.read_with(cx, |state, _| assert!(!state.is_visible()));

        blink.update(cx, |state, cx| {
            let mut steady = target;
            steady.cursor.blinking = false;
            state.sync(Some(steady), cx);
        });
        blink.read_with(cx, |state, _| {
            assert!(state.is_visible());
            assert!(!state.is_running());
        });

        cx.executor().advance_clock(TERMINAL_CURSOR_BLINK_INTERVAL);
        cx.executor().run_until_parked();
        blink.read_with(cx, |state, _| {
            assert!(state.is_visible());
            assert!(!state.is_running());
        });
    }
}
