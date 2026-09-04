use gpui_kit::component::{
    Sizable, Size, StyleSized as _,
    input::{Editor, EditorState, Input, InputState, Textarea, TextareaState},
    label::Label,
};
use gpui_kit::{
    App, Entity, EntityId, Focusable as _, Global, IntoElement, ParentElement, Pixels, RenderOnce,
    SharedString, StyleRefinement, Styled, TextRun, Window, black, div,
    prelude::FluentBuilder as _, px, rems, rgb,
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
    register_input_hint_with_layout(input.entity_id(), hint, InputHintLayout::Plain, cx);
}

pub(crate) fn register_textarea_hint(
    input: &Entity<TextareaState>,
    hint: impl Into<SharedString>,
    cx: &mut App,
) {
    register_input_hint_with_layout(input.entity_id(), hint, InputHintLayout::Plain, cx);
}

pub(crate) fn register_code_editor_input_hint(
    input: &Entity<EditorState>,
    hint: impl Into<SharedString>,
    folding: bool,
    cx: &mut App,
) {
    register_input_hint_with_layout(
        input.entity_id(),
        hint,
        InputHintLayout::CodeEditor { folding },
        cx,
    );
}

fn register_input_hint_with_layout(
    id: EntityId,
    hint: impl Into<SharedString>,
    layout: InputHintLayout,
    cx: &mut App,
) {
    init(cx);
    let hint = hint.into();
    let registry = cx.global_mut::<InputHintRegistry>();

    if hint.is_empty() {
        registry.hints.remove(&id);
    } else {
        registry.hints.insert(id, InputHintEntry { hint, layout });
    }
}

fn input_hint(id: EntityId, cx: &App) -> Option<InputHintEntry> {
    if !cx.has_global::<InputHintRegistry>() {
        return None;
    }

    cx.global::<InputHintRegistry>().hints.get(&id).cloned()
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

#[allow(clippy::too_many_arguments)]
fn render_hinted_control(
    control: impl IntoElement,
    hint: Option<InputHintEntry>,
    is_empty: bool,
    is_focused: bool,
    size: Size,
    hint_left: Option<Pixels>,
    hint_right: Option<Pixels>,
    hint_top: Option<Pixels>,
    hint_bottom: Option<Pixels>,
    hint_center_y: bool,
    hide_hint_on_focus: bool,
    container_h_full: bool,
    window: &mut Window,
) -> impl IntoElement {
    let default_hint_x = size.input_px();
    let default_hint_y = size.input_py();
    let hint_left = hint_left.unwrap_or(default_hint_x);
    let hint_right = hint_right.unwrap_or(default_hint_x);
    let hint_top = hint_top.unwrap_or(default_hint_y);
    let hint_bottom = hint_bottom.unwrap_or(default_hint_y);

    div()
        .relative()
        .w_full()
        .when(container_h_full, |this| this.h_full())
        .child(control)
        .when(is_empty && (!hide_hint_on_focus || !is_focused), |this| {
            this.when_some(hint, |this, hint| {
                let gutter_width = match hint.layout {
                    InputHintLayout::Plain => px(0.0),
                    InputHintLayout::CodeEditor { folding } => {
                        code_editor_gutter_width(size, folding, window)
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
                        .when(hint_center_y, |this| this.items_center())
                        .overflow_hidden()
                        .child(
                            Label::new(hint.hint)
                                .input_text_size(size)
                                .text_color(rgb(input_hint_foreground())),
                        ),
                )
            })
        })
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
    hide_hint_on_focus: bool,
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
            hide_hint_on_focus: false,
            container_h_full: false,
        }
    }

    pub(crate) fn appearance(mut self, appearance: bool) -> Self {
        self.input = self.input.appearance(appearance);
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

    pub(crate) fn hide_hint_on_focus(mut self) -> Self {
        self.hide_hint_on_focus = true;
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
        let hint = input_hint(self.state.entity_id(), cx);
        let is_empty = self.state.read(cx).value().is_empty();
        let is_focused = self.state.focus_handle(cx).is_focused(window);
        render_hinted_control(
            self.input,
            hint,
            is_empty,
            is_focused,
            self.size,
            self.hint_left,
            self.hint_right,
            self.hint_top,
            self.hint_bottom,
            self.hint_center_y,
            self.hide_hint_on_focus,
            self.container_h_full,
            window,
        )
    }
}

#[derive(IntoElement)]
pub(crate) struct HintedTextarea {
    textarea: Textarea,
    state: Entity<TextareaState>,
    hint_left: Option<Pixels>,
    hint_top: Option<Pixels>,
    hint_bottom: Option<Pixels>,
}

impl HintedTextarea {
    pub(crate) fn new(state: &Entity<TextareaState>) -> Self {
        Self {
            textarea: Textarea::new(state).bordered(false).border_0(),
            state: state.clone(),
            hint_left: None,
            hint_top: None,
            hint_bottom: None,
        }
    }

    pub(crate) fn appearance(mut self, appearance: bool) -> Self {
        self.textarea = self.textarea.appearance(appearance);
        self
    }

    pub(crate) fn hint_left(mut self, left: Pixels) -> Self {
        self.hint_left = Some(left);
        self
    }

    pub(crate) fn hint_top(mut self, top: Pixels) -> Self {
        self.hint_top = Some(top);
        self
    }

    pub(crate) fn hint_bottom(mut self, bottom: Pixels) -> Self {
        self.hint_bottom = Some(bottom);
        self
    }
}

impl Styled for HintedTextarea {
    fn style(&mut self) -> &mut StyleRefinement {
        self.textarea.style()
    }
}

impl RenderOnce for HintedTextarea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let hint = input_hint(self.state.entity_id(), cx);
        let is_empty = self.state.read(cx).value().is_empty();
        let is_focused = self.state.focus_handle(cx).is_focused(window);
        render_hinted_control(
            self.textarea,
            hint,
            is_empty,
            is_focused,
            Size::default(),
            self.hint_left,
            None,
            self.hint_top,
            self.hint_bottom,
            false,
            false,
            false,
            window,
        )
    }
}

#[derive(IntoElement)]
pub(crate) struct HintedEditor {
    editor: Editor,
    state: Entity<EditorState>,
    hint_top: Option<Pixels>,
    hint_bottom: Option<Pixels>,
    container_h_full: bool,
}

impl HintedEditor {
    pub(crate) fn new(state: &Entity<EditorState>) -> Self {
        Self {
            editor: Editor::new(state).bordered(false).border_0(),
            state: state.clone(),
            hint_top: None,
            hint_bottom: None,
            container_h_full: false,
        }
    }

    pub(crate) fn appearance(mut self, appearance: bool) -> Self {
        self.editor = self.editor.appearance(appearance);
        self
    }

    pub(crate) fn hint_top(mut self, top: Pixels) -> Self {
        self.hint_top = Some(top);
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

impl Styled for HintedEditor {
    fn style(&mut self) -> &mut StyleRefinement {
        self.editor.style()
    }
}

impl RenderOnce for HintedEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let hint = input_hint(self.state.entity_id(), cx);
        let is_empty = self.state.read(cx).value().is_empty();
        let is_focused = self.state.focus_handle(cx).is_focused(window);
        render_hinted_control(
            self.editor,
            hint,
            is_empty,
            is_focused,
            Size::default(),
            None,
            None,
            self.hint_top,
            self.hint_bottom,
            true,
            false,
            self.container_h_full,
            window,
        )
    }
}
