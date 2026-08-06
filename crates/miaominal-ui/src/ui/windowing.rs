use std::rc::Rc;

use anyhow::{Result, anyhow};
use gpui::{AnyWindowHandle, App, Entity, Global, WeakEntity};

use super::AppView;

type DetachedWindowOpener = dyn Fn(&mut App) -> Result<DetachedWindowTarget>;

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

pub fn register_detached_window_opener(
    cx: &mut App,
    opener: impl Fn(&mut App) -> Result<DetachedWindowTarget> + 'static,
) {
    cx.set_global(DetachedWindowFactory {
        opener: Rc::new(opener),
    });
}

pub(crate) fn open_detached_window(cx: &mut App) -> Result<DetachedWindowTarget> {
    let opener = cx
        .try_global::<DetachedWindowFactory>()
        .map(|factory| factory.opener.clone())
        .ok_or_else(|| anyhow!("detached window opener is unavailable"))?;
    let target = opener(cx)?;
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
