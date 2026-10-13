//! Retained callbacks from a feature-owned overlay only target its original surface.

use gpui_kit::{App, Context, Entity};
use std::rc::Rc;

pub(in crate::ui) struct OverlayTarget<T: 'static> {
    entity: Entity<T>,
    is_current: Rc<dyn Fn(&T) -> bool>,
}

impl<T: 'static> Clone for OverlayTarget<T> {
    fn clone(&self) -> Self {
        Self {
            entity: self.entity.clone(),
            is_current: self.is_current.clone(),
        }
    }
}

impl<T: 'static> OverlayTarget<T> {
    pub(in crate::ui) fn new(entity: Entity<T>, is_current: impl Fn(&T) -> bool + 'static) -> Self {
        Self {
            entity,
            is_current: Rc::new(is_current),
        }
    }

    pub(in crate::ui) fn update<R>(
        &self,
        cx: &mut App,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> Option<R> {
        let is_current = self.is_current.clone();
        self.entity
            .update(cx, |owner, cx| is_current(owner).then(|| update(owner, cx)))
    }
}
