use crate::ui::assets::AppIcon;
use gpui_kit::component::{
    Icon, Sizable as _, h_flex,
    select::{SearchableVec, SelectItem},
    v_flex,
};
use gpui_kit::{AnyElement, App, SharedString, Window, div, prelude::*, rgb};
use miaominal_core::profile::SessionProfile;

/// Shared connection choice used by forwarding and managed-key deployment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) struct ProfileSelectItem {
    id: String,
    title: SharedString,
    summary: SharedString,
}

impl ProfileSelectItem {
    pub(in crate::ui) fn new(profile: &SessionProfile) -> Self {
        Self {
            id: profile.id.clone(),
            title: SharedString::from(profile.name.clone()),
            summary: SharedString::from(profile.summary()),
        }
    }
}

impl SelectItem for ProfileSelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;
        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Subheading.scaled())
                    .text_color(rgb(roles.on_surface))
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(roles.on_surface_variant))
                    .child(self.summary.clone()),
            )
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        query.is_empty()
            || format!("{} {}", self.title.as_ref(), self.summary.as_ref())
                .to_ascii_lowercase()
                .contains(&query)
    }
}

#[derive(Clone, Debug)]
pub(in crate::ui) struct SelectOption<T> {
    title: SharedString,
    value: T,
    icon: Option<AppIcon>,
    disabled: bool,
}

impl<T> SelectOption<T> {
    pub(in crate::ui) fn new(value: T, title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            value,
            icon: None,
            disabled: false,
        }
    }

    pub(in crate::ui) fn value(&self) -> &T {
        &self.value
    }

    pub(in crate::ui) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
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

    fn disabled(&self) -> bool {
        self.disabled
    }
}

/// Managed-key choice shared by the host editor and key deployment controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) struct ManagedKeySelectItem {
    id: String,
    title: SharedString,
    summary: SharedString,
}

impl ManagedKeySelectItem {
    pub(in crate::ui) fn new(key: &miaominal_core::keychain::ManagedKeyRecord) -> Self {
        Self {
            id: key.id.clone(),
            title: SharedString::from(key.name.clone()),
            summary: SharedString::from(format!("{}  {}", key.id, key.algorithm)),
        }
    }
    pub(in crate::ui) fn sorted_items(
        keys: &[miaominal_core::keychain::ManagedKeyRecord],
    ) -> Vec<Self> {
        let mut items = keys.iter().map(Self::new).collect::<Vec<_>>();
        items.sort_by(|left, right| {
            left.title
                .as_ref()
                .to_ascii_lowercase()
                .cmp(&right.title.as_ref().to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        items
    }
    pub(in crate::ui) fn options(
        keys: &[miaominal_core::keychain::ManagedKeyRecord],
    ) -> SearchableVec<Self> {
        SearchableVec::new(Self::sorted_items(keys))
    }
}

impl SelectItem for ManagedKeySelectItem {
    type Value = String;
    fn title(&self) -> SharedString {
        self.title.clone()
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;
        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Subheading.scaled())
                    .text_color(rgb(roles.on_surface))
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(roles.on_surface_variant))
                    .child(self.summary.clone()),
            )
    }
    fn value(&self) -> &Self::Value {
        &self.id
    }
    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        query.is_empty()
            || format!("{} {}", self.title.as_ref(), self.summary.as_ref())
                .to_ascii_lowercase()
                .contains(&query)
    }
}
