use crate::ui::components::catalog_card::{
    catalog_card_chip, catalog_card_chips, catalog_card_content, catalog_card_icon_content,
    catalog_card_title,
};
use crate::ui::components::shell_tokens::{ShellColors, ShellMetrics, space};
use crate::ui::i18n;
use gpui_kit::component::{
    Icon, IconName, Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex,
};
use gpui_kit::{AnyElement, App, IntoElement, SharedString, Window, div, prelude::*};
use miaominal_core::profile::SessionProfile;

fn visible_tags(tags: &[String]) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for tag in tags
        .iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
    {
        if !result.iter().any(|item| item.eq_ignore_ascii_case(tag)) {
            result.push(tag.to_owned());
        }
    }
    result
}

pub(in crate::ui::features::hosts) fn host_card(
    id: SharedString,
    profile: &SessionProfile,
    _list: bool,
    editing: bool,
    more: AnyElement,
    cx: &App,
    on_connect: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let colors = ShellColors::current();
    // The profile open in the editor keeps a persistent selected fill.
    let surface = if editing {
        colors.selected
    } else {
        colors.card
    };
    let hover = if editing {
        colors.selected
    } else {
        colors.hover
    };
    let tags = visible_tags(&profile.tags);
    // Groups are optional: an ungrouped host simply has no subtitle.
    let subtitle: Option<SharedString> = if profile.is_local() {
        Some(i18n::string("hosts.home.local").into())
    } else if profile.group.trim().is_empty() {
        None
    } else {
        Some(profile.group.clone().into())
    };
    let has_tags = !tags.is_empty();
    let title = h_flex()
        .min_w_0()
        .gap(space(4.0))
        .child(catalog_card_title(profile.name.clone()))
        .when(profile.is_favorite, |row| {
            row.child(
                Icon::new(IconName::Star)
                    .small()
                    .flex_shrink_0()
                    .text_color(colors.primary),
            )
        });
    // Reserve room for the overlaid "more" button.
    let more_spacer = div()
        .flex_shrink_0()
        .w(space(ShellMetrics::CARD_ICON_SIZE))
        .into_any_element();
    let tags_row = has_tags.then(|| {
        catalog_card_chips()
            .children(
                tags.iter()
                    .take(2)
                    .map(|tag| catalog_card_chip(tag.clone(), None)),
            )
            .when(tags.len() > 2, |row| {
                row.child(catalog_card_chip(format!("+{}", tags.len() - 2), None).flex_shrink_0())
            })
            .into_any_element()
    });
    let content = catalog_card_content(
        catalog_card_icon_content(crate::ui::assets::terminal_icon().small()),
        title,
        subtitle,
        Some(more_spacer),
        tags_row,
    );
    let name = profile.name.clone();
    let tooltip_tags = tags.join(", ");
    div()
        .w_full()
        .min_w_0()
        .relative()
        .rounded(space(ShellMetrics::CARD_RADIUS))
        .bg(surface)
        .child(
            Button::new(SharedString::from(format!("{id}:connect")))
                .ghost()
                .w_full()
                .h(space(ShellMetrics::CARD_HEIGHT))
                .px(space(ShellMetrics::CARD_PADDING))
                .py_0()
                .items_center()
                .rounded(miaominal_settings::scaled_font_size(
                    ShellMetrics::CARD_RADIUS,
                ))
                .bg(surface)
                .text_color(colors.foreground)
                .custom(
                    ButtonCustomVariant::new(cx)
                        .color(surface)
                        .foreground(colors.foreground)
                        .hover(hover)
                        .active(colors.selected),
                )
                .accessibility_label(i18n::string_args(
                    "hosts.home.connect_name",
                    &[("name", &name)],
                ))
                // Right-click only opens the card's context menu. Without this the
                // press focuses the button, and the menu restores that focus on
                // dismiss, leaving a focus ring on the card.
                .on_mouse_down(gpui_kit::MouseButton::Right, |_, window, _| {
                    window.prevent_default()
                })
                .child(
                    div()
                        .id(SharedString::from(format!("{id}:tags")))
                        .w_full()
                        .min_w_0()
                        .when(has_tags, |body| {
                            body.tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(tooltip_tags.clone())
                                    .build(window, cx)
                            })
                        })
                        .child(content),
                )
                .on_click(move |_, window, cx| on_connect(window, cx)),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}:menu-region")))
                .occlude()
                .on_click(|_, _, cx| cx.stop_propagation())
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation()
                })
                .absolute()
                .right(space(ShellMetrics::CARD_PADDING))
                // Centered on the card, like the icon.
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .child(more),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tags_keep_full_text_and_deduplicate() {
        assert_eq!(
            visible_tags(&[
                " dev ".into(),
                "DEV".into(),
                "".into(),
                "long tag without character clipping".into()
            ]),
            vec!["dev", "long tag without character clipping"]
        );
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use gpui_kit::{
        Context, Entity, Render, TestAppContext, component::Root, test::TestWindowExt as _,
    };
    use std::{cell::Cell, rc::Rc};
    struct CardHarness {
        profile: SessionProfile,
        connects: Rc<Cell<usize>>,
        more: Rc<Cell<usize>>,
    }
    impl Render for CardHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let more = self.more.clone();
            let connects = self.connects.clone();
            host_card(
                "fixture".into(),
                &self.profile,
                false,
                false,
                Button::new("fixture-more")
                    .label("More")
                    .on_click(move |_, _, _| more.set(more.get() + 1))
                    .into_any_element(),
                cx,
                move |_, _| connects.set(connects.get() + 1),
            )
        }
    }
    #[gpui_kit::test]
    fn card_height_is_identical_with_optional_group_and_tags(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut expected_height = None;
        for (group, tags) in [
            ("", vec![]),
            ("Production", vec![]),
            ("", vec!["dev".to_string()]),
            ("Production", vec!["dev".to_string(), "ssh".to_string()]),
        ] {
            let (_, window_cx) = cx.add_window_view(move |window, cx| {
                let mut profile = SessionProfile::blank("fixture", 0);
                profile.name = "Fixture host".into();
                profile.group = group.into();
                profile.tags = tags;
                let view = cx.new(|_| CardHarness {
                    profile,
                    connects: Rc::new(Cell::new(0)),
                    more: Rc::new(Cell::new(0)),
                });
                Root::new(view, window, cx)
            });
            window_cx.update(|window, cx| {
                window.render_frame(cx);
                let height = window.find("fixture:connect").bounds().size.height;
                if let Some(expected) = expected_height {
                    assert_eq!(
                        height, expected,
                        "group and tags must not change card height"
                    );
                } else {
                    expected_height = Some(height);
                }
            });
        }
    }

    #[gpui_kit::test]
    fn card_main_and_more_have_independent_hit_regions(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let connects = Rc::new(Cell::new(0));
        let more = Rc::new(Cell::new(0));
        let output = connects.clone();
        let more_output = more.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let mut profile = SessionProfile::blank("fixture", 0);
            profile.name = "Fixture host".into();
            let view: Entity<CardHarness> = cx.new(|_| CardHarness {
                profile,
                connects: output,
                more: more_output,
            });
            Root::new(view, window, cx)
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("fixture-more", cx);
            assert_eq!(more.get(), 1);
            assert_eq!(connects.get(), 0);
            window.click("fixture:connect", cx);
            assert_eq!(connects.get(), 1);
            assert_eq!(more.get(), 1);
            let bounds = window.find("fixture:connect").bounds();
            let initial_scale = window.scale_factor();
            for scale in [1.5, 2.0] {
                window.set_scale_factor(scale);
                window.render_frame(cx);
                let scaled = window.find("fixture:connect").bounds();
                assert_eq!(scaled.origin, bounds.origin);
                assert_eq!(scaled.size.width, bounds.size.width);
                // The two text lines snap separately to device pixels. Their
                // combined natural height can vary by two physical pixels.
                let height_tolerance = 2.0 / scale.min(initial_scale);
                assert!(
                    (f32::from(scaled.size.height - bounds.size.height)).abs()
                        <= height_tolerance + 0.001,
                    "scale {scale}: baseline {bounds:?}, observed {scaled:?}",
                );
                window.click("fixture-more", cx);
                assert_eq!(connects.get(), 1);
            }
            assert_eq!(more.get(), 3);
        });
    }

    #[gpui_kit::test]
    fn right_click_does_not_focus_the_card(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let mut profile = SessionProfile::blank("fixture", 0);
            profile.name = "Fixture host".into();
            let view: Entity<CardHarness> = cx.new(|_| CardHarness {
                profile,
                connects: Rc::new(Cell::new(0)),
                more: Rc::new(Cell::new(0)),
            });
            Root::new(view, window, cx)
        });
        let center = cx.update(|window, cx| {
            window.render_frame(cx);
            window.find("fixture:connect").bounds().center()
        });
        cx.simulate_mouse_down(center, gpui_kit::MouseButton::Right, Default::default());
        cx.simulate_mouse_up(center, gpui_kit::MouseButton::Right, Default::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(
                window.focused(cx).is_none(),
                "a right-click must not leave focus (and its ring) on the card"
            );
        });
    }
}
