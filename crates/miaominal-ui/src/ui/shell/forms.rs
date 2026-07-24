use super::*;

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct SelectOption<T> {
    title: SharedString,
    value: T,
    icon: Option<AppIcon>,
}

impl<T> SelectOption<T> {
    pub(in crate::ui::shell) fn new(value: T, title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            value,
            icon: None,
        }
    }

    pub(in crate::ui::shell) fn value(&self) -> &T {
        &self.value
    }
}

impl<T: Clone + PartialEq> SelectItem for SelectOption<T> {
    type Value = T;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn display_title(&self) -> Option<AnyElement> {
        self.icon.map(|icon| {
            h_flex()
                .gap_2()
                .items_center()
                .child(Icon::new(icon).small())
                .child(self.title.clone())
                .into_any_element()
        })
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        if let Some(icon) = self.icon {
            h_flex()
                .gap_2()
                .items_center()
                .child(Icon::new(icon).small())
                .child(self.title.clone())
                .into_any_element()
        } else {
            div().child(self.title.clone()).into_any_element()
        }
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct TerminalSearchAnimation {
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
    pub(in crate::ui::shell) from: f32,
    pub(in crate::ui::shell) to: f32,
}

pub(in crate::ui::shell) struct WorkspaceForms {
    pub(in crate::ui::shell) rename_input: Entity<InputState>,
}
