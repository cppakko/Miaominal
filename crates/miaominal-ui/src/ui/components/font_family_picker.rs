use gpui_kit::component::{
    IndexPath, Sizable as _, Size, StyleSized as _,
    combobox::{Combobox, ComboboxState},
    searchable_list::SearchableVec,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, Styled as _, TestSupportExt as _, Window, div, px, rgb,
};

pub(crate) type FontFamilyPickerState = ComboboxState<SearchableVec<String>>;

pub(crate) fn new_font_family_picker(
    options: &[String],
    selected: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<FontFamilyPickerState> {
    let selected_index = options
        .iter()
        .position(|font| font.eq_ignore_ascii_case(selected))
        .map(|index| IndexPath::default().row(index));
    cx.new(|cx| {
        ComboboxState::new(
            SearchableVec::new(options.to_vec()),
            selected_index.into_iter().collect(),
            window,
            cx,
        )
        .searchable(true)
    })
}

/// Synchronizing a committed setting must not emit another user change.
pub(crate) fn sync_font_family_picker(
    state: &Entity<FontFamilyPickerState>,
    options: &[String],
    selected: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let mut options = options.to_vec();
    let index = options
        .iter()
        .position(|font| font.eq_ignore_ascii_case(selected))
        .unwrap_or_else(|| {
            options.push(selected.to_owned());
            options.len() - 1
        });
    state.update(cx, |state, cx| {
        state.set_items(SearchableVec::new(options), window, cx);
        state.set_query("", window, cx);
        state.set_selected_indices([IndexPath::default().row(index)], window, cx);
    });
}

pub(crate) fn font_family_picker(
    id: impl Into<SharedString>,
    state: &Entity<FontFamilyPickerState>,
    size: Size,
    _cx: &App,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    div()
        .id(id.into())
        .test_support()
        .w_full()
        .input_h(size)
        .child(
            Combobox::new(state)
                .with_size(size)
                .w_full()
                .appearance(false)
                .rounded(px(14.0))
                .bg(rgb(roles.surface_container_highest))
                .text_color(rgb(roles.on_surface))
                .search_placeholder(crate::ui::i18n::string(
                    "settings.appearance.font_picker.search_placeholder",
                ))
                .empty(|_, _| {
                    crate::ui::components::empty_state::EmptyState::new(
                        crate::ui::assets::AppIcon::Search,
                        crate::ui::i18n::string("settings.appearance.font_picker.no_matches"),
                    )
                    .size(crate::ui::components::empty_state::EmptyStateSize::Inline)
                    .build()
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{Root, button::Button, combobox::ComboboxEvent, v_flex};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Context, IntoElement, Render, Subscription, TestAppContext, px};

    struct FontPickerFixture {
        state: Entity<FontFamilyPickerState>,
        terminal: Entity<FontFamilyPickerState>,
        options: Vec<String>,
        committed: String,
        changes: usize,
        _subscription: Subscription,
    }

    impl FontPickerFixture {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            let options = [
                "Cascadia Code",
                "Consolas",
                "JetBrains Mono",
                "Missing user font",
            ]
            .map(str::to_owned)
            .to_vec();
            let state = new_font_family_picker(&options, "Missing user font", window, cx);
            let terminal = new_font_family_picker(&options, "Consolas", window, cx);
            let subscription = cx.subscribe(&state, |this, _, event, cx| {
                if let ComboboxEvent::Change(values) = event {
                    this.committed = values[0].clone();
                    this.changes += 1;
                    cx.notify();
                }
            });
            Self {
                state,
                terminal,
                options,
                committed: "Missing user font".into(),
                changes: 0,
                _subscription: subscription,
            }
        }
    }

    impl Render for FontPickerFixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .w(px(280.0))
                .gap_2()
                .child(font_family_picker(
                    "font-picker",
                    &self.state,
                    Size::Medium,
                    cx,
                ))
                .child(font_family_picker(
                    "terminal-picker",
                    &self.terminal,
                    Size::Medium,
                    cx,
                ))
                .child(
                    Button::new("reset-font")
                        .label("Reset")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.committed = "Cascadia Code".into();
                            sync_font_family_picker(
                                &this.state,
                                &this.options,
                                &this.committed,
                                window,
                                cx,
                            );
                            cx.notify();
                        })),
                )
        }
    }

    #[gpui_kit::test]
    fn font_family_picker_filters_commits_and_restores_focus(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut fixture = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| FontPickerFixture::new(window, cx));
            fixture = Some(view.clone());
            Root::new(view, window, cx)
        });
        let fixture = fixture.unwrap();
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert_eq!(
                fixture.read(cx).state.read(cx).selected_value().as_deref(),
                Some("Missing user font")
            );
            window.click("font-picker", cx);
            window.input("JetBrains", cx);
            assert_eq!(
                fixture.read(cx).state.read(cx).query(cx).as_ref(),
                "JetBrains"
            );
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        });
        cx.update(|window, cx| {
            assert_eq!(fixture.read(cx).committed, "JetBrains Mono");
            assert_eq!(fixture.read(cx).changes, 1);
            assert_eq!(
                fixture
                    .read(cx)
                    .terminal
                    .read(cx)
                    .selected_value()
                    .as_deref(),
                Some("Consolas")
            );
            assert!(
                gpui_kit::Focusable::focus_handle(&fixture.read(cx).state, cx).is_focused(window)
            );

            window.click("font-picker", cx);
            window.input("no matching font", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.press("enter", cx);
            window.press("escape", cx);
        });
        cx.update(|window, cx| {
            assert_eq!(
                fixture.read(cx).changes,
                1,
                "empty results and cancellation cannot save a font"
            );
            assert!(
                gpui_kit::Focusable::focus_handle(&fixture.read(cx).state, cx).is_focused(window)
            );
            window.click("reset-font", cx);
            assert_eq!(
                fixture.read(cx).state.read(cx).selected_value().as_deref(),
                Some("Cascadia Code")
            );
            assert_eq!(
                fixture.read(cx).changes,
                1,
                "owner synchronization must not feed back a user change"
            );
            assert!(fixture.read(cx).state.read(cx).query(cx).is_empty());
            let state = fixture.read(cx).state.clone();
            let options = fixture.read(cx).options.clone();
            sync_font_family_picker(&state, &options, "Imported font", window, cx);
            assert_eq!(
                state.read(cx).selected_value().as_deref(),
                Some("Imported font")
            );
            assert_eq!(fixture.read(cx).changes, 1);
            assert_eq!(
                fixture
                    .read(cx)
                    .terminal
                    .read(cx)
                    .selected_value()
                    .as_deref(),
                Some("Consolas")
            );
            for mode in [
                gpui_kit::component::ThemeMode::Light,
                gpui_kit::component::ThemeMode::Dark,
            ] {
                gpui_kit::component::Theme::change(mode, Some(window), cx);
                for scale in [1.0, 1.5, 2.0] {
                    window.set_scale_factor(scale);
                    window.render_frame(cx);
                    assert!(window.find("font-picker").bounds().size.height > px(0.0));
                    assert!(window.find("font-picker").bounds().size.width <= px(280.0));
                }
            }
        });
    }
}
