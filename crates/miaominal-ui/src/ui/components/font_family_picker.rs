use super::{TextInputSurface, surface_text_input};
use crate::ui::assets::AppIcon;
use gpui::{
    AnyElement, App, Entity, Focusable as _, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    UniformListScrollHandle, Window, div, prelude::FluentBuilder as _, px, rgb, uniform_list,
};
use gpui_component::{
    ElementExt as _, Icon, Sizable as _, Size, StyleSized as _,
    button::{Button, ButtonVariants as _},
    input::InputState,
    popover::Popover,
    scroll::ScrollableElement as _,
    v_flex,
};
use std::{cell::Cell, rc::Rc};

pub(crate) struct FontFamilyPickerState {
    id: SharedString,
    query_input: Entity<InputState>,
    scroll_handle: UniformListScrollHandle,
    no_matches: Option<SharedString>,
}

impl FontFamilyPickerState {
    pub(crate) fn new(
        id: impl Into<SharedString>,
        query_input: Entity<InputState>,
        scroll_handle: UniformListScrollHandle,
    ) -> Self {
        Self {
            id: id.into(),
            query_input,
            scroll_handle,
            no_matches: None,
        }
    }

    pub(crate) fn no_matches(mut self, no_matches: impl Into<SharedString>) -> Self {
        self.no_matches = Some(no_matches.into());
        self
    }
}

pub(crate) fn font_family_picker(
    state: FontFamilyPickerState,
    selected: impl Into<SharedString>,
    options: Vec<String>,
    size: Size,
    on_select: impl Fn(String, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let FontFamilyPickerState {
        id,
        query_input,
        scroll_handle,
        no_matches,
    } = state;
    let selected = selected.into();
    let roles = miaominal_settings::current_theme().material.roles;
    let query_focus = query_input.focus_handle(cx);
    let trigger_width = Rc::new(Cell::new(px(320.0)));
    let captured_trigger_width = trigger_width.clone();
    let content_trigger_width = trigger_width;
    let content_id = id.clone();
    let trigger_id = SharedString::from(format!("{id}-trigger"));
    let on_select = Rc::new(on_select);
    let no_matches = no_matches.unwrap_or_else(|| {
        crate::ui::i18n::string("settings.appearance.font_picker.no_matches").into()
    });

    let trigger = Button::new(trigger_id)
        .ghost()
        .dropdown_caret(true)
        .with_size(size)
        .input_h(size)
        .w_full()
        .rounded(px(14.0))
        .border_0()
        .bg(rgb(roles.surface_container_highest))
        .text_color(rgb(roles.on_surface))
        .label(selected.clone())
        .on_prepaint(move |bounds, _, _| {
            captured_trigger_width.set(bounds.size.width);
        });

    div()
        .w_full()
        .child(
            Popover::new(id)
                .appearance(false)
                .track_focus(&query_focus)
                .trigger(trigger)
                .content(move |_, _, cx| {
                    let query = query_input.read(cx).value().to_string();
                    let normalized_query = query.trim().to_lowercase();
                    let filtered_options = Rc::new(
                        options
                            .iter()
                            .filter(|font| {
                                normalized_query.is_empty()
                                    || font.to_lowercase().contains(&normalized_query)
                            })
                            .cloned()
                            .collect::<Vec<_>>(),
                    );
                    let popover = cx.entity();
                    let item_count = filtered_options.len();
                    let list_height = px((item_count.max(1) as f32 * 40.0).min(280.0));
                    let list_id = SharedString::from(format!("{content_id}-options-list"));
                    let row_options = filtered_options.clone();
                    let row_selected = selected.clone();
                    let row_content_id = content_id.clone();
                    let row_on_select = on_select.clone();
                    let row_query_input = query_input.clone();
                    let row_popover = popover.clone();
                    let no_matches = no_matches.clone();
                    let option_list =
                        uniform_list(list_id, item_count, move |visible_range, _, _| {
                            visible_range
                                .map(|index| {
                                    let font = row_options[index].clone();
                                    let is_selected =
                                        font.eq_ignore_ascii_case(row_selected.as_ref());
                                    let option_id = SharedString::from(format!(
                                        "{row_content_id}-option-{index}-{}",
                                        font.to_lowercase()
                                    ));
                                    let on_select = row_on_select.clone();
                                    let query_input = row_query_input.clone();
                                    let popover = row_popover.clone();
                                    let font_for_click = font.clone();
                                    let hover_background = if is_selected {
                                        roles.secondary_container
                                    } else {
                                        roles.surface_container_highest
                                    };

                                    div()
                                        .id(option_id)
                                        .w_full()
                                        .h(px(40.0))
                                        .flex()
                                        .items_center()
                                        .px_3()
                                        .rounded(px(10.0))
                                        .cursor_pointer()
                                        .text_color(rgb(if is_selected {
                                            roles.on_secondary_container
                                        } else {
                                            roles.on_surface
                                        }))
                                        .when(is_selected, |this| {
                                            this.bg(rgb(roles.secondary_container))
                                        })
                                        .hover(move |this| this.bg(rgb(hover_background)))
                                        .child(font)
                                        .on_click(move |_, window, cx| {
                                            on_select(font_for_click.clone(), window, cx);
                                            query_input.update(cx, |input, cx| {
                                                input.set_value("", window, cx);
                                            });
                                            popover
                                                .update(cx, |state, cx| state.dismiss(window, cx));
                                        })
                                })
                                .collect::<Vec<_>>()
                        })
                        .size_full()
                        .track_scroll(&scroll_handle);

                    let options_content = if item_count == 0 {
                        div()
                            .h(list_height)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(roles.on_surface_variant))
                            .child(no_matches)
                            .into_any_element()
                    } else {
                        div()
                            .relative()
                            .h(list_height)
                            .child(option_list)
                            .vertical_scrollbar(&scroll_handle)
                            .into_any_element()
                    };

                    v_flex()
                        .w(content_trigger_width.get())
                        .max_w_full()
                        .gap_1()
                        .p_1()
                        .rounded(px(14.0))
                        .border_1()
                        .border_color(rgb(roles.outline_variant))
                        .bg(rgb(roles.surface_container))
                        .text_color(rgb(roles.on_surface))
                        .shadow_lg()
                        .child(
                            surface_text_input(&query_input, TextInputSurface::Highest)
                                .with_size(size)
                                .hint_left(px(42.0))
                                .hide_hint_on_focus()
                                .prefix(
                                    div()
                                        .flex()
                                        .items_center()
                                        .text_color(rgb(roles.on_surface_variant))
                                        .child(Icon::new(AppIcon::Search).small()),
                                ),
                        )
                        .child(options_content)
                }),
        )
        .into_any_element()
}
