//! Window overlay policy and opaque owner-rendered snapshots.
//!
//! Features capture private prompt data in callbacks. The window Root owns
//! layering, focus trapping/restoration and the exit clock.

use gpui_kit::{AnyElement, App, FocusHandle, Window};
use std::{collections::HashMap, fmt, rc::Rc};

type OverlayRenderer = dyn Fn(Option<f32>, f32, &App) -> AnyElement;
type OverlayDismissal = dyn Fn(&mut Window, &mut App);

#[derive(Clone)]
pub(in crate::ui) struct OverlaySnapshot {
    key: String,
    revision: u64,
    render: Rc<OverlayRenderer>,
    dismiss: Rc<OverlayDismissal>,
    submit: Option<Rc<OverlayDismissal>>,
    keyboard_focus: Option<FocusHandle>,
    escape_dismissible: bool,
}

impl fmt::Debug for OverlaySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OverlaySnapshot")
            .field("key", &self.key)
            .field("revision", &self.revision)
            .field("escape_dismissible", &self.escape_dismissible)
            .finish_non_exhaustive()
    }
}

impl OverlaySnapshot {
    pub(in crate::ui) fn new(
        key: impl Into<String>,
        revision: u64,
        render: impl Fn(Option<f32>, f32, &App) -> AnyElement + 'static,
        dismiss: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            key: key.into(),
            revision,
            render: Rc::new(render),
            dismiss: Rc::new(dismiss),
            submit: None,
            keyboard_focus: None,
            escape_dismissible: true,
        }
    }

    pub(in crate::ui) fn with_keyboard_focus(mut self, focus: FocusHandle) -> Self {
        self.keyboard_focus = Some(focus);
        self
    }

    pub(in crate::ui) fn escape_dismissible(mut self, dismissible: bool) -> Self {
        self.escape_dismissible = dismissible;
        self
    }

    pub(in crate::ui) fn with_submit(
        mut self,
        submit: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.submit = Some(Rc::new(submit));
        self
    }

    pub(in crate::ui) fn stable_key(&self) -> &str {
        &self.key
    }
    pub(in crate::ui) fn revision(&self) -> u64 {
        self.revision
    }
    pub(in crate::ui) fn keyboard_focus(&self) -> Option<&FocusHandle> {
        self.keyboard_focus.as_ref()
    }
    pub(in crate::ui) fn is_escape_dismissible(&self) -> bool {
        self.escape_dismissible
    }

    pub(in crate::ui) fn render(
        &self,
        exit_progress: Option<f32>,
        viewport_height: f32,
        cx: &App,
    ) -> AnyElement {
        (self.render)(exit_progress, viewport_height, cx)
    }

    pub(in crate::ui) fn dismiss(&self, window: &mut Window, cx: &mut App) {
        (self.dismiss)(window, cx);
    }

    pub(in crate::ui) fn submit(&self, window: &mut Window, cx: &mut App) -> bool {
        if let Some(submit) = &self.submit {
            submit(window, cx);
            true
        } else {
            false
        }
    }
}

/// Each feature owns its generations; replacing one surface does not invalidate another.
#[derive(Default)]
pub(in crate::ui) struct OverlayRevisionState {
    next: u64,
    generations: HashMap<&'static str, u64>,
}

impl OverlayRevisionState {
    pub(in crate::ui) fn begin(&mut self, surface: &'static str) -> u64 {
        self.next = self.next.wrapping_add(1);
        self.generations.insert(surface, self.next);
        self.next
    }

    pub(in crate::ui) fn current(&self, surface: &'static str) -> u64 {
        self.generations.get(surface).copied().unwrap_or_default()
    }
}
