use gpui_kit::component::{FocusableExt as _, switch::Switch};
use gpui_kit::{App, ElementId, IntoElement, RenderOnce, SharedString, Styled, Window, rgb};

#[derive(IntoElement)]
pub(crate) struct Md3Switch {
    inner: Switch,
}

impl Md3Switch {
    pub(crate) fn checked(mut self, checked: bool) -> Self {
        self.inner = self.inner.checked(checked);
        self
    }

    pub(crate) fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.inner = self.inner.tooltip(tooltip);
        self
    }

    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.inner = self.inner.on_click(handler);
        self
    }
}

impl RenderOnce for Md3Switch {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        self.inner.focus_ring(window.last_input_was_keyboard())
    }
}

pub(crate) fn md3_switch(id: impl Into<ElementId>) -> Md3Switch {
    let material = miaominal_settings::current_theme().material;
    Md3Switch {
        inner: Switch::new(id)
            .color(rgb(material.roles.primary))
            .rounded_3xl(),
    }
}
