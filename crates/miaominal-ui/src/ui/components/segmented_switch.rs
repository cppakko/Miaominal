use gpui::{
    Animation, AnimationExt as _, App, ElementId, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement as _, RenderOnce, SharedString, Styled, Window, div, ease_in_out,
    prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::h_flex;
use std::{rc::Rc, time::Duration};

const DEFAULT_WIDTH: f32 = 204.0;
const DEFAULT_HEIGHT: f32 = 34.0;
const DEFAULT_PADDING: f32 = 2.0;
const ANIMATION_DURATION: Duration = Duration::from_millis(180);

type SegmentedSwitchClick = dyn Fn(usize, &mut Window, &mut App);

#[derive(IntoElement)]
pub(crate) struct SegmentedSwitch {
    id: ElementId,
    items: Vec<SharedString>,
    selected_index: usize,
    width: f32,
    height: f32,
    padding: f32,
    on_click: Option<Rc<SegmentedSwitchClick>>,
}

impl SegmentedSwitch {
    pub(crate) fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            items: Vec::new(),
            selected_index: 0,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            padding: DEFAULT_PADDING,
            on_click: None,
        }
    }

    pub(crate) fn selected_index(mut self, selected_index: usize) -> Self {
        self.selected_index = selected_index;
        self
    }

    pub(crate) fn item(mut self, label: impl Into<SharedString>) -> Self {
        self.items.push(label.into());
        self
    }

    pub(crate) fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub(crate) fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub(crate) fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for SegmentedSwitch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let item_count = self.items.len();
        let selected_index = if item_count == 0 {
            0
        } else {
            self.selected_index.min(item_count - 1)
        };
        let inner_width = (self.width - self.padding * 2.0).max(0.0);
        let segment_width = if item_count == 0 {
            inner_width
        } else {
            inner_width / item_count as f32
        };
        let segment_height = (self.height - self.padding * 2.0).max(0.0);
        let selected_left = self.padding + segment_width * selected_index as f32;
        let state_id: ElementId = (self.id.clone(), "selected-index").into();
        let selected_state = window.use_keyed_state(state_id, cx, |_, _| selected_index);
        let previous_index = *selected_state.read(cx);

        let mut root = h_flex()
            .id(self.id.clone())
            .relative()
            .w(px(self.width))
            .h(px(self.height))
            .flex_none()
            .items_center()
            .justify_center()
            .p(px(self.padding))
            .rounded(px(999.0))
            .overflow_hidden()
            .bg(rgb(roles.surface_container_high));

        if item_count > 0 {
            let indicator = div()
                .absolute()
                .top(px(self.padding))
                .w(px(segment_width))
                .h(px(segment_height))
                .rounded(px(999.0))
                .bg(rgb(roles.secondary_container));

            let indicator = if previous_index != selected_index {
                let from_left =
                    self.padding + segment_width * previous_index.min(item_count - 1) as f32;
                let to_left = selected_left;
                let selected_state = selected_state.clone();
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(ANIMATION_DURATION).await;
                    selected_state.update(cx, |state, _| *state = selected_index);
                })
                .detach();

                indicator
                    .left(px(from_left))
                    .with_animation(
                        ElementId::from((
                            self.id.clone(),
                            format!("indicator-to-{selected_index}"),
                        )),
                        Animation::new(ANIMATION_DURATION).with_easing(ease_in_out),
                        move |element, delta| {
                            element.left(px(from_left + (to_left - from_left) * delta))
                        },
                    )
                    .into_any_element()
            } else {
                indicator.left(px(selected_left)).into_any_element()
            };

            root = root.child(indicator);
        }

        for (index, label) in self.items.into_iter().enumerate() {
            let selected = index == selected_index;
            let foreground = if selected {
                roles.on_secondary_container
            } else {
                text_muted
            };
            let mut item = div()
                .w(px(segment_width))
                .h(px(segment_height))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .min_w(px(0.0))
                .overflow_hidden()
                .rounded(px(999.0))
                .text_color(rgb(foreground))
                .cursor_pointer()
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .text_center()
                        .text_ellipsis()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .line_height(miaominal_settings::scaled_line_height(16.0))
                        .when(selected, |this| this.font_weight(FontWeight::SEMIBOLD))
                        .child(label),
                );

            if !selected {
                item = item.hover(move |this| this.bg(rgb(roles.surface_container_highest)));
            }

            if let Some(on_click) = self.on_click.clone() {
                item = item.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    on_click(index, window, cx)
                });
            }

            root = root.child(item);
        }

        root
    }
}
