use gpui::{
    App, Entity, EntityId, Global, IntoElement, ParentElement, Pixels, RenderOnce, SharedString,
    StyleRefinement, Styled, TextRun, Window, black, div, prelude::FluentBuilder as _, px, rems,
    rgb,
};
use gpui_component::{
    Sizable, Size, StyleSized as _,
    input::{Input, InputState},
    label::Label,
};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct InputHintRegistry {
    hints: HashMap<EntityId, InputHintEntry>,
}

impl Global for InputHintRegistry {}

#[derive(Clone, Copy)]
enum InputHintLayout {
    Plain,
    CodeEditor { folding: bool },
}

#[derive(Clone)]
struct InputHintEntry {
    hint: SharedString,
    layout: InputHintLayout,
}

pub(crate) fn init(cx: &mut App) {
    if !cx.has_global::<InputHintRegistry>() {
        cx.set_global(InputHintRegistry::default());
    }
}

pub(crate) fn register_input_hint(
    input: &Entity<InputState>,
    hint: impl Into<SharedString>,
    cx: &mut App,
) {
    register_input_hint_with_layout(input, hint, InputHintLayout::Plain, cx);
}

pub(crate) fn register_code_editor_input_hint(
    input: &Entity<InputState>,
    hint: impl Into<SharedString>,
    folding: bool,
    cx: &mut App,
) {
    register_input_hint_with_layout(input, hint, InputHintLayout::CodeEditor { folding }, cx);
}

fn register_input_hint_with_layout(
    input: &Entity<InputState>,
    hint: impl Into<SharedString>,
    layout: InputHintLayout,
    cx: &mut App,
) {
    init(cx);
    let hint = hint.into();
    let id = input.entity_id();
    let registry = cx.global_mut::<InputHintRegistry>();

    if hint.is_empty() {
        registry.hints.remove(&id);
    } else {
        registry.hints.insert(id, InputHintEntry { hint, layout });
    }
}

fn input_hint(input: &Entity<InputState>, cx: &App) -> Option<InputHintEntry> {
    if !cx.has_global::<InputHintRegistry>() {
        return None;
    }

    cx.global::<InputHintRegistry>()
        .hints
        .get(&input.entity_id())
        .cloned()
}

fn input_text_size(size: Size, window: &Window) -> Pixels {
    match size {
        Size::XSmall => rems(0.75).to_pixels(window.rem_size()),
        Size::Small | Size::Medium => rems(0.875).to_pixels(window.rem_size()),
        Size::Large => rems(1.0).to_pixels(window.rem_size()),
        Size::Size(size) => size * 0.875,
    }
}

fn code_editor_gutter_width(size: Size, folding: bool, window: &mut Window) -> Pixels {
    const LINE_NUMBER_LEN: usize = 5;
    const LINE_NUMBER_RIGHT_MARGIN: Pixels = px(10.0);
    const FOLD_ICON_HITBOX_WIDTH: Pixels = px(18.0);

    let style = window.text_style();
    let font_size = input_text_size(size, window);
    let line_number = "+".repeat(LINE_NUMBER_LEN);
    let shaped_line = window.text_system().shape_line(
        SharedString::from(line_number),
        font_size,
        &[TextRun {
            len: LINE_NUMBER_LEN,
            font: style.font(),
            color: black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );

    shaped_line.width
        + LINE_NUMBER_RIGHT_MARGIN
        + if folding {
            FOLD_ICON_HITBOX_WIDTH
        } else {
            px(0.0)
        }
}

pub(crate) fn input_hint_foreground() -> u32 {
    let material = miaominal_settings::current_theme().material;
    crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 68 } else { 46 },
    )
}

#[derive(IntoElement)]
pub(crate) struct HintedInput {
    input: Input,
    state: Entity<InputState>,
    size: Size,
    hint_left: Option<Pixels>,
    hint_right: Option<Pixels>,
    hint_top: Option<Pixels>,
    hint_bottom: Option<Pixels>,
    hint_center_y: bool,
    container_h_full: bool,
}

impl HintedInput {
    pub(crate) fn new(state: &Entity<InputState>) -> Self {
        Self {
            input: Input::new(state).focus_bordered(false).border_0(),
            state: state.clone(),
            size: Size::default(),
            hint_left: None,
            hint_right: None,
            hint_top: None,
            hint_bottom: None,
            hint_center_y: true,
            container_h_full: false,
        }
    }

    pub(crate) fn appearance(mut self, appearance: bool) -> Self {
        self.input = self.input.appearance(appearance);
        self
    }

    pub(crate) fn focus_bordered(mut self, bordered: bool) -> Self {
        self.input = self.input.focus_bordered(bordered);
        self
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.input = self.input.disabled(disabled);
        self
    }

    pub(crate) fn prefix(mut self, prefix: impl IntoElement) -> Self {
        self.input = self.input.prefix(prefix);
        self
    }

    pub(crate) fn suffix(mut self, suffix: impl IntoElement) -> Self {
        self.input = self.input.suffix(suffix);
        self
    }

    pub(crate) fn hint_left(mut self, left: Pixels) -> Self {
        self.hint_left = Some(left);
        self
    }

    pub(crate) fn hint_top(mut self, top: Pixels) -> Self {
        self.hint_top = Some(top);
        self.hint_center_y = false;
        self
    }

    pub(crate) fn hint_bottom(mut self, bottom: Pixels) -> Self {
        self.hint_bottom = Some(bottom);
        self
    }

    pub(crate) fn container_h_full(mut self) -> Self {
        self.container_h_full = true;
        self
    }
}

impl Sizable for HintedInput {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        let size = size.into();
        self.input = self.input.with_size(size);
        self.size = size;
        self
    }
}

impl Styled for HintedInput {
    fn style(&mut self) -> &mut StyleRefinement {
        self.input.style()
    }
}

impl RenderOnce for HintedInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let hint = input_hint(&self.state, cx);
        let is_empty = self.state.read(cx).value().is_empty();
        let default_hint_x = self.size.input_px();
        let default_hint_y = self.size.input_py();
        let hint_left = self.hint_left.unwrap_or(default_hint_x);
        let hint_right = self.hint_right.unwrap_or(default_hint_x);
        let hint_top = self.hint_top.unwrap_or(default_hint_y);
        let hint_bottom = self.hint_bottom.unwrap_or(default_hint_y);

        div()
            .relative()
            .w_full()
            .when(self.container_h_full, |this| this.h_full())
            .child(self.input)
            .when(is_empty, |this| {
                this.when_some(hint, |this, hint| {
                    let gutter_width = match hint.layout {
                        InputHintLayout::Plain => px(0.0),
                        InputHintLayout::CodeEditor { folding } => {
                            code_editor_gutter_width(self.size, folding, window)
                        }
                    };

                    this.child(
                        div()
                            .absolute()
                            .left(hint_left + gutter_width)
                            .right(hint_right)
                            .top(hint_top)
                            .bottom(hint_bottom)
                            .flex()
                            .when(self.hint_center_y, |this| this.items_center())
                            .overflow_hidden()
                            .child(
                                Label::new(hint.hint)
                                    .input_text_size(self.size)
                                    .text_color(rgb(input_hint_foreground())),
                            ),
                    )
                })
            })
    }
}
