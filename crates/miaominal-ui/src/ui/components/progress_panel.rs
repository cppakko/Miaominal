//! Shared progress-panel geometry and transitions; each feature retains its own state.
use crate::ui::components::animation::CONTAINER_TRANSITION_DURATION;
use gpui_kit::{Pixels, Window};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProgressPanelTransitionPhase {
    Entering,
    Exiting,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProgressPanelTransition {
    pub(crate) phase: ProgressPanelTransitionPhase,
    pub(crate) started_at: Instant,
    pub(crate) duration: Duration,
}

#[derive(Debug, Clone)]
pub(crate) struct ProgressPanelDrag {
    pub(crate) initial_pointer: f32,
    pub(crate) initial_flex: f32,
    pub(crate) container_height: Pixels,
}

pub(crate) fn update_progress_center_state(
    current_visible: &mut bool,
    transition: &mut Option<ProgressPanelTransition>,
    visible: bool,
    started_at: Instant,
) -> bool {
    let phase = if visible {
        ProgressPanelTransitionPhase::Entering
    } else {
        ProgressPanelTransitionPhase::Exiting
    };
    if *current_visible == visible
        && transition
            .as_ref()
            .is_none_or(|transition| transition.phase == phase)
    {
        return false;
    }

    *current_visible = visible;
    *transition = Some(ProgressPanelTransition {
        phase,
        started_at,
        duration: CONTAINER_TRANSITION_DURATION,
    });
    true
}

pub(crate) fn progress_center_render_visibility(
    visible: bool,
    transition: &mut Option<ProgressPanelTransition>,
    window: &mut Window,
) -> Option<f32> {
    let Some(current_transition) = *transition else {
        return visible.then_some(1.0);
    };

    let duration_seconds = current_transition.duration.as_secs_f32();
    if duration_seconds <= f32::EPSILON {
        *transition = None;
        return visible.then_some(1.0);
    }

    let elapsed = Instant::now().saturating_duration_since(current_transition.started_at);
    let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
    let eased = progress * progress * (3.0 - 2.0 * progress);

    if progress >= 1.0 {
        *transition = None;
        return visible.then_some(1.0);
    }

    window.request_animation_frame();

    Some(match current_transition.phase {
        ProgressPanelTransitionPhase::Entering => eased,
        ProgressPanelTransitionPhase::Exiting => 1.0 - eased,
    })
}
