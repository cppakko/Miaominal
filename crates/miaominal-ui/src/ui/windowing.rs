use std::rc::Rc;

use anyhow::{Result, anyhow};
use gpui_kit::{
    AnyWindowHandle, App, Bounds, DisplayId, Entity, Global, Pixels, Point, Size, WeakEntity,
    WindowId, point, px, size,
};

use super::AppView;

const DEFAULT_DETACHED_WINDOW_WIDTH: f32 = 1240.0;
const DEFAULT_DETACHED_WINDOW_HEIGHT: f32 = 800.0;
const MIN_DETACHED_WINDOW_WIDTH: f32 = 720.0;
const MIN_DETACHED_WINDOW_HEIGHT: f32 = 480.0;
const TAB_DETACH_WINDOW_CURSOR_OFFSET_X: f32 = 120.0;
const TAB_DETACH_WINDOW_CURSOR_OFFSET_Y: f32 = 28.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetachedWindowPlacement {
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
}

impl DetachedWindowPlacement {
    pub fn new(bounds: Bounds<Pixels>, display_id: Option<DisplayId>) -> Self {
        Self { bounds, display_id }
    }

    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    pub fn display_id(&self) -> Option<DisplayId> {
        self.display_id
    }
}

type DetachedWindowOpener =
    dyn Fn(Option<DetachedWindowPlacement>, &mut App) -> Result<DetachedWindowTarget>;

pub struct DetachedWindowTarget {
    window: AnyWindowHandle,
    view: Entity<AppView>,
}

impl DetachedWindowTarget {
    pub fn new(window: AnyWindowHandle, view: Entity<AppView>) -> Self {
        Self { window, view }
    }

    pub(crate) fn into_parts(self) -> (AnyWindowHandle, Entity<AppView>) {
        (self.window, self.view)
    }
}

struct DetachedWindowFactory {
    opener: Rc<DetachedWindowOpener>,
}

impl Global for DetachedWindowFactory {}

#[derive(Clone)]
struct DetachedWindowRegistration {
    window: AnyWindowHandle,
    view: WeakEntity<AppView>,
}

#[derive(Default)]
struct DetachedWindowRegistry {
    windows: Vec<DetachedWindowRegistration>,
}

impl Global for DetachedWindowRegistry {}

#[derive(Clone)]
pub(crate) struct AppWindowRegistration {
    pub(crate) window: AnyWindowHandle,
    pub(crate) view: WeakEntity<AppView>,
}

#[derive(Default)]
struct AppWindowRegistry {
    windows: Vec<AppWindowRegistration>,
}

impl Global for AppWindowRegistry {}

pub(crate) fn register_app_window(
    window: AnyWindowHandle,
    view: WeakEntity<AppView>,
    cx: &mut App,
) {
    let window_id = window.window_id();
    let registration = AppWindowRegistration { window, view };
    if cx.has_global::<AppWindowRegistry>() {
        let windows = &mut cx.global_mut::<AppWindowRegistry>().windows;
        windows.retain(|entry| {
            entry.view.upgrade().is_some() && entry.window.window_id() != window_id
        });
        windows.push(registration);
    } else {
        cx.set_global(AppWindowRegistry {
            windows: vec![registration],
        });
    }
}

pub(crate) fn live_app_windows(cx: &App) -> Vec<AppWindowRegistration> {
    cx.try_global::<AppWindowRegistry>()
        .map(|registry| {
            registry
                .windows
                .iter()
                .filter(|entry| entry.view.upgrade().is_some())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn remove_detached_window_registration(window_id: WindowId, cx: &mut App) {
    if cx.has_global::<DetachedWindowRegistry>() {
        cx.global_mut::<DetachedWindowRegistry>()
            .windows
            .retain(|entry| {
                entry.view.upgrade().is_some() && entry.window.window_id() != window_id
            });
    }
}

pub(crate) fn unregister_app_window(window_id: WindowId, cx: &mut App) {
    if cx.has_global::<AppWindowRegistry>() {
        cx.global_mut::<AppWindowRegistry>()
            .windows
            .retain(|entry| {
                entry.view.upgrade().is_some() && entry.window.window_id() != window_id
            });
    }
    remove_detached_window_registration(window_id, cx);
}

pub(crate) fn next_other_live_app_window(
    excluding: WindowId,
    cx: &App,
) -> Option<AppWindowRegistration> {
    live_app_windows(cx)
        .into_iter()
        .find(|entry| entry.window.window_id() != excluding && entry.view.upgrade().is_some())
}

pub fn register_detached_window_opener(
    cx: &mut App,
    opener: impl Fn(Option<DetachedWindowPlacement>, &mut App) -> Result<DetachedWindowTarget> + 'static,
) {
    cx.set_global(DetachedWindowFactory {
        opener: Rc::new(opener),
    });
}

pub(crate) fn compute_detached_window_placement(
    source_bounds: Bounds<Pixels>,
    preferred_size: Size<Pixels>,
    drop_position: Point<Pixels>,
    displays: &[(DisplayId, Bounds<Pixels>)],
) -> Option<DetachedWindowPlacement> {
    if displays.is_empty() {
        return None;
    }

    let drop_screen = point(
        source_bounds.origin.x + drop_position.x,
        source_bounds.origin.y + drop_position.y,
    );

    let (display_id, display_bounds) = displays
        .iter()
        .copied()
        .find(|(_, bounds)| bounds.contains(&drop_screen))
        .unwrap_or_else(|| {
            let drop_x = f32::from(drop_screen.x);
            let drop_y = f32::from(drop_screen.y);
            displays
                .iter()
                .copied()
                .min_by(|(_, a), (_, b)| {
                    let dist_sq = |bounds: Bounds<Pixels>| {
                        let left = f32::from(bounds.origin.x);
                        let top = f32::from(bounds.origin.y);
                        let right = left + f32::from(bounds.size.width).max(0.0);
                        let bottom = top + f32::from(bounds.size.height).max(0.0);
                        let dx = drop_x - drop_x.clamp(left, right);
                        let dy = drop_y - drop_y.clamp(top, bottom);
                        dx * dx + dy * dy
                    };
                    dist_sq(*a).total_cmp(&dist_sq(*b))
                })
                .expect("displays is non-empty")
        });

    let preferred_w = f32::from(preferred_size.width);
    let preferred_h = f32::from(preferred_size.height);
    let (base_w, base_h) = if preferred_w > 0.0 && preferred_h > 0.0 {
        (preferred_w, preferred_h)
    } else {
        (
            DEFAULT_DETACHED_WINDOW_WIDTH,
            DEFAULT_DETACHED_WINDOW_HEIGHT,
        )
    };

    let display_w = f32::from(display_bounds.size.width).max(1.0);
    let display_h = f32::from(display_bounds.size.height).max(1.0);
    let width = base_w.clamp(MIN_DETACHED_WINDOW_WIDTH.min(display_w), display_w);
    let height = base_h.clamp(MIN_DETACHED_WINDOW_HEIGHT.min(display_h), display_h);

    let left = f32::from(display_bounds.origin.x);
    let top = f32::from(display_bounds.origin.y);
    let max_x = (left + display_w - width).max(left);
    let max_y = (top + display_h - height).max(top);
    let x = (f32::from(drop_screen.x) - TAB_DETACH_WINDOW_CURSOR_OFFSET_X).clamp(left, max_x);
    let y = (f32::from(drop_screen.y) - TAB_DETACH_WINDOW_CURSOR_OFFSET_Y).clamp(top, max_y);

    Some(DetachedWindowPlacement::new(
        Bounds::new(point(px(x), px(y)), size(px(width), px(height))),
        Some(display_id),
    ))
}

pub(crate) fn open_detached_window(
    placement: Option<DetachedWindowPlacement>,
    cx: &mut App,
) -> Result<DetachedWindowTarget> {
    let opener = cx
        .try_global::<DetachedWindowFactory>()
        .map(|factory| factory.opener.clone())
        .ok_or_else(|| anyhow!("detached window opener is unavailable"))?;
    let target = opener(placement, cx)?;
    let registration = DetachedWindowRegistration {
        window: target.window,
        view: target.view.downgrade(),
    };
    if cx.has_global::<DetachedWindowRegistry>() {
        cx.global_mut::<DetachedWindowRegistry>()
            .windows
            .push(registration);
    } else {
        cx.set_global(DetachedWindowRegistry {
            windows: vec![registration],
        });
    }
    Ok(target)
}

pub(crate) fn prepare_detached_windows_for_application_quit(cx: &mut App) {
    let registrations = cx
        .try_global::<DetachedWindowRegistry>()
        .map(|registry| registry.windows.clone())
        .unwrap_or_default();

    for registration in registrations {
        let Some(view) = registration.view.upgrade() else {
            continue;
        };
        if let Err(error) = registration.window.update(cx, move |_, window, cx| {
            view.update(cx, |view, cx| {
                view.prepare_detached_window_close(window, cx);
            });
        }) {
            log::debug!("failed to prepare detached window before application quit: {error:?}");
        }
    }
}
use crate::ui::workspace::transfer::WorkspaceTransferExt;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_positions_window_near_cursor_on_single_display() {
        let displays = vec![(
            DisplayId::new(1),
            Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0))),
        )];
        let source_bounds = Bounds::new(point(px(100.0), px(100.0)), size(px(1240.0), px(800.0)));
        let placement = compute_detached_window_placement(
            source_bounds,
            size(px(1000.0), px(700.0)),
            point(px(400.0), px(-40.0)),
            &displays,
        )
        .expect("single display should yield placement");

        assert_eq!(placement.display_id(), Some(DisplayId::new(1)));
        // drop_screen = (500, 60), minus offset (120, 28) => (380, 32)
        assert_eq!(
            placement.bounds(),
            Bounds::new(point(px(380.0), px(32.0)), size(px(1000.0), px(700.0)))
        );
    }

    #[test]
    fn placement_clamps_to_top_left_and_bottom_right_display_edges() {
        let displays = vec![(
            DisplayId::new(1),
            Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0))),
        )];
        let source_bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(1240.0), px(800.0)));

        let top_left = compute_detached_window_placement(
            source_bounds,
            size(px(1000.0), px(700.0)),
            point(px(20.0), px(-10.0)),
            &displays,
        )
        .expect("top-left edge placement");
        assert_eq!(top_left.bounds().origin, point(px(0.0), px(0.0)));

        let bottom_right = compute_detached_window_placement(
            source_bounds,
            size(px(1000.0), px(700.0)),
            point(px(1900.0), px(1060.0)),
            &displays,
        )
        .expect("bottom-right edge placement");
        assert_eq!(bottom_right.bounds().origin, point(px(920.0), px(380.0)));
    }

    #[test]
    fn placement_selects_target_display_and_clamps_within_it() {
        let displays = vec![
            (
                DisplayId::new(1),
                Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0))),
            ),
            (
                DisplayId::new(2),
                Bounds::new(point(px(1920.0), px(0.0)), size(px(1440.0), px(900.0))),
            ),
        ];
        let source_bounds = Bounds::new(point(px(800.0), px(100.0)), size(px(1240.0), px(800.0)));

        // drop_screen = (2500, 300) inside display 2
        let placement = compute_detached_window_placement(
            source_bounds,
            size(px(1200.0), px(800.0)),
            point(px(1700.0), px(200.0)),
            &displays,
        )
        .expect("second monitor placement");
        assert_eq!(placement.display_id(), Some(DisplayId::new(2)));
        assert_eq!(
            placement.bounds(),
            Bounds::new(point(px(2160.0), px(100.0)), size(px(1200.0), px(800.0)))
        );

        // Near right edge of display 2: drop_screen = (3350, 880)
        let clamped = compute_detached_window_placement(
            source_bounds,
            size(px(1200.0), px(800.0)),
            point(px(2550.0), px(780.0)),
            &displays,
        )
        .expect("clamped second monitor placement");
        assert_eq!(clamped.display_id(), Some(DisplayId::new(2)));
        assert_eq!(clamped.bounds().origin, point(px(2160.0), px(100.0)));
    }

    #[test]
    fn placement_returns_none_when_no_displays_exist() {
        let source_bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(1240.0), px(800.0)));
        assert!(
            compute_detached_window_placement(
                source_bounds,
                size(px(1240.0), px(800.0)),
                point(px(-50.0), px(20.0)),
                &[],
            )
            .is_none()
        );
    }

    #[test]
    fn placement_clamps_oversized_preferred_size_to_display_while_respecting_minimum() {
        let displays = vec![(
            DisplayId::new(1),
            Bounds::new(point(px(0.0), px(0.0)), size(px(1024.0), px(640.0))),
        )];
        let source_bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(1600.0), px(1000.0)));

        let oversized = compute_detached_window_placement(
            source_bounds,
            size(px(1600.0), px(1000.0)),
            point(px(300.0), px(200.0)),
            &displays,
        )
        .expect("oversized placement");
        assert_eq!(oversized.bounds().size, size(px(1024.0), px(640.0)));

        let tiny = compute_detached_window_placement(
            source_bounds,
            size(px(300.0), px(200.0)),
            point(px(300.0), px(200.0)),
            &displays,
        )
        .expect("undersized placement");
        assert_eq!(tiny.bounds().size, size(px(720.0), px(480.0)));
    }
}
