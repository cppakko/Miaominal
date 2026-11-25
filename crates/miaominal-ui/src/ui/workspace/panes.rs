use crate::ui::contracts::{PaneId, SplitAxis, TabId};
use crate::ui::features::session::{
    TerminalHoveredLink, TerminalKeyboardState, TerminalLinkQuery, TerminalMouseGesture,
    TerminalMouseReportingState, TerminalScrollbarDrag,
};
use gpui_kit::{Bounds, FocusHandle, Pixels, Point};
use miaominal_terminal::{terminal_cell_width_default, terminal_line_height_default};
use std::time::{Duration, Instant};
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct PaneSplitDragMarker {
    pub path: Vec<usize>,
    pub child_index: usize,
    pub axis: SplitAxis,
}

impl gpui_kit::Render for PaneSplitDragMarker {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        _cx: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::Styled;
        // Invisible 1x1 ghost so the cursor styling drives the visual feedback.
        gpui_kit::div().size(gpui_kit::px(1.0))
    }
}

#[derive(Clone)]
pub(crate) struct PaneSplitDragState {
    pub path: Vec<usize>,
    pub child_index: usize,
    pub axis: SplitAxis,
    pub initial_pointer: f32,
    pub initial_flex_a: f32,
    pub initial_flex_b: f32,
    pub container_size: f32,
}

#[derive(Clone, Debug)]
pub(crate) enum PaneSplitAnimationKind {
    Opening,
    Closing,
}

#[derive(Clone, Debug)]
pub(crate) struct PaneCloseAnimation {
    pub removed_pane_id: PaneId,
    pub hidden_tab_id: Option<TabId>,
}

#[derive(Clone, Debug)]
pub(crate) struct PaneSplitAnimation {
    pub kind: PaneSplitAnimationKind,
    pub path: Vec<usize>,
    pub child_index: usize,
    pub new_child_index: usize,
    pub axis: SplitAxis,
    pub from_flex_a: f32,
    pub from_flex_b: f32,
    pub to_flex_a: f32,
    pub to_flex_b: f32,
    pub started_at: Instant,
    pub duration: Duration,
    pub pending_close: Option<PaneCloseAnimation>,
}

/// Transient interpolation state for zooming a pane in or out within a split layout.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PaneZoomAnimation {
    pub pane_id: PaneId,
    pub from: f32,
    pub to: f32,
    pub started_at: Instant,
    pub duration: Duration,
}

impl PaneZoomAnimation {
    pub fn new(
        pane_id: PaneId,
        from: f32,
        to: f32,
        started_at: Instant,
        base_duration: Duration,
    ) -> Self {
        let from = from.clamp(0.0, 1.0);
        let to = to.clamp(0.0, 1.0);
        let distance = (to - from).abs();
        let duration = if distance <= f32::EPSILON {
            Duration::ZERO
        } else {
            base_duration.mul_f32(distance.clamp(0.25, 1.0))
        };
        Self {
            pane_id,
            from,
            to,
            started_at,
            duration,
        }
    }

    pub fn raw_progress(&self, now: Instant) -> f32 {
        let duration_seconds = self.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            return 1.0;
        }
        (now.saturating_duration_since(self.started_at).as_secs_f32() / duration_seconds)
            .clamp(0.0, 1.0)
    }

    pub fn sample(&self, now: Instant) -> f32 {
        let p = self.raw_progress(now);
        let eased = p * p * (3.0 - 2.0 * p);
        (self.from + (self.to - self.from) * eased).clamp(0.0, 1.0)
    }

    pub fn is_finished(&self, now: Instant) -> bool {
        self.raw_progress(now) >= 1.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneTabDropZone {
    Center,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PaneTabDropTarget {
    pub pane_id: PaneId,
    pub zone: PaneTabDropZone,
}

pub(crate) struct ParkedPane {
    pub active_tab: Option<TabId>,
    pub terminal_focus: FocusHandle,
    pub terminal_bounds: Option<Bounds<Pixels>>,
    pub terminal_cell_width: f32,
    pub terminal_line_height: f32,
    pub terminal_mouse_gesture: Option<TerminalMouseGesture>,
    pub terminal_keyboard: TerminalKeyboardState,
    pub terminal_mouse_reporting: TerminalMouseReportingState,
    pub terminal_pointer_target: Option<TabId>,
    pub terminal_pointer_position: Option<Point<Pixels>>,
    pub terminal_link_query: Option<TerminalLinkQuery>,
    pub terminal_hovered_link: Option<TerminalHoveredLink>,
    pub terminal_link_open_modifier: bool,
    pub terminal_scrollbar_drag: Option<TerminalScrollbarDrag>,
    pub terminal_scrollbar_last_interaction_at: Option<Instant>,
}

impl ParkedPane {
    #[allow(dead_code)]
    pub fn empty(focus: FocusHandle) -> Self {
        Self {
            active_tab: None,
            terminal_focus: focus,
            terminal_bounds: None,
            terminal_cell_width: terminal_cell_width_default(),
            terminal_line_height: terminal_line_height_default(),
            terminal_mouse_gesture: None,
            terminal_keyboard: TerminalKeyboardState::default(),
            terminal_mouse_reporting: TerminalMouseReportingState::default(),
            terminal_pointer_target: None,
            terminal_pointer_position: None,
            terminal_link_query: None,
            terminal_hovered_link: None,
            terminal_link_open_modifier: false,
            terminal_scrollbar_drag: None,
            terminal_scrollbar_last_interaction_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_zoom_animation_samples_smoothstep_and_completes() {
        let start = Instant::now();
        let base = Duration::from_millis(180);
        let anim = PaneZoomAnimation::new(PaneId(1), 0.0, 1.0, start, base);
        assert_eq!(anim.duration, base);
        assert_eq!(anim.sample(start), 0.0);
        assert!(!anim.is_finished(start));

        let mid = start + Duration::from_millis(90);
        assert!((anim.sample(mid) - 0.5).abs() < 1e-5);
        assert!(!anim.is_finished(mid));

        let end = start + base;
        assert_eq!(anim.sample(end), 1.0);
        assert!(anim.is_finished(end));
    }

    #[test]
    fn pane_zoom_animation_zero_distance_completes_immediately() {
        let start = Instant::now();
        let anim = PaneZoomAnimation::new(PaneId(2), 1.0, 1.0, start, Duration::from_millis(180));
        assert_eq!(anim.duration, Duration::ZERO);
        assert_eq!(anim.sample(start), 1.0);
        assert!(anim.is_finished(start));
    }

    #[test]
    fn pane_zoom_animation_reverses_from_sampled_value_with_scaled_duration() {
        let start = Instant::now();
        let base = Duration::from_millis(200);
        let forward = PaneZoomAnimation::new(PaneId(1), 0.0, 1.0, start, base);
        let interrupt_at = start + Duration::from_millis(100);
        let sampled = forward.sample(interrupt_at);
        assert!((sampled - 0.5).abs() < 1e-5);

        let reverse = PaneZoomAnimation::new(PaneId(1), sampled, 0.0, interrupt_at, base);
        assert!((reverse.from - 0.5).abs() < 1e-5);
        assert_eq!(reverse.to, 0.0);
        assert_eq!(reverse.duration, Duration::from_millis(100));
        assert!((reverse.sample(interrupt_at) - 0.5).abs() < 1e-5);
        assert!(!reverse.is_finished(interrupt_at + Duration::from_millis(50)));
        assert_eq!(
            reverse.sample(interrupt_at + Duration::from_millis(100)),
            0.0
        );
        assert!(reverse.is_finished(interrupt_at + Duration::from_millis(100)));

        // Small distance clamps duration floor to 25% of base_duration.
        let tiny_reverse = PaneZoomAnimation::new(PaneId(1), 0.1, 0.0, interrupt_at, base);
        assert_eq!(tiny_reverse.duration, Duration::from_millis(50));
    }
}
