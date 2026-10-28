#[cfg(test)]
use gpui_kit::InteractiveElement as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable, Size, StyleSized as _,
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

fn code_editor_gutter_width(
    line_count: usize,
    font_size: Pixels,
    font_family: &SharedString,
    folding: bool,
    window: &mut Window,
) -> Pixels {
    // Mirrors gpui-base's `layout_line_numbers`: three digits are reserved for
    // small documents, growing up to seven, followed by a 6px margin. A foldable
    // editor adds the fold icon hitbox.
    const MIN_LINE_NUMBER_DIGITS: usize = 3;
    const MAX_LINE_NUMBER_DIGITS: usize = 7;
    const LINE_NUMBER_RIGHT_MARGIN: Pixels = px(6.0);
    const FOLD_ICON_HITBOX_WIDTH: Pixels = px(18.0);

    let digits = (line_count.max(1).ilog10() as usize + 1)
        .clamp(MIN_LINE_NUMBER_DIGITS, MAX_LINE_NUMBER_DIGITS);
    let mut font = window.text_style().font();
    font.family = font_family.clone();
    let shaped_line = window.text_system().shape_line(
        SharedString::from("+".repeat(digits)),
        font_size,
        &[TextRun {
            len: digits,
            font,
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
    text_metrics: Option<(Pixels, Pixels)>,
    hint_font: Option<SharedString>,
    hint_color: Option<gpui_kit::Hsla>,
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
                let element = div()
                    .absolute()
                    .left(hint_left)
                    .right(hint_right)
                    .top(hint_top)
                    // Single-line hints share the input's vertical center;
                    // multiline hints stay within the first line's box.
                    .when_some(
                        text_metrics.filter(|_| !hint_center_y),
                        |this, (_, height)| this.h(height),
                    )
                    .when(hint_center_y || text_metrics.is_none(), |this| {
                        this.bottom(hint_bottom)
                    })
                    .flex()
                    .when(hint_center_y, |this| this.items_center())
                    .overflow_hidden()
                    .child(
                        Label::new(hint.hint)
                            .input_text_size(size)
                            .when_some(hint_font, |label, family| label.font_family(family))
                            .when_some(text_metrics, |label, (size, height)| {
                                label.text_size(size).line_height(height)
                            })
                            .text_color(
                                hint_color.unwrap_or_else(|| rgb(input_hint_foreground()).into()),
                            ),
                    );
                #[cfg(test)]
                let element = element.debug_selector(|| "input-hint".into());
                this.child(element)
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
    text_metrics: Option<(Pixels, Pixels)>,
    hint_color: Option<gpui_kit::Hsla>,
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
            text_metrics: None,
            hint_color: None,
        }
    }

    pub(crate) fn appearance(mut self, appearance: bool) -> Self {
        self.input = self.input.appearance(appearance);
        self
    }

    /// Apply the same typography to entered text and its separately drawn hint.
    pub(crate) fn text_metrics(mut self, size: Pixels, line_height: Pixels) -> Self {
        self.input = self.input.text_size(size).line_height(line_height);
        self.text_metrics = Some((size, line_height));
        self
    }

    pub(crate) fn hint_color(mut self, color: gpui_kit::Hsla) -> Self {
        self.hint_color = Some(color);
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
            self.text_metrics,
            None,
            self.hint_color,
        )
    }
}

#[derive(IntoElement)]
pub(crate) struct HintedTextarea {
    textarea: Textarea,
    state: Entity<TextareaState>,
    outer_padding: Pixels,
}

impl HintedTextarea {
    pub(crate) fn new(state: &Entity<TextareaState>) -> Self {
        Self {
            textarea: Textarea::new(state).bordered(false).border_0(),
            state: state.clone(),
            outer_padding: px(0.0),
        }
    }

    pub(crate) fn appearance(mut self, appearance: bool) -> Self {
        self.textarea = self.textarea.appearance(appearance);
        self
    }

    /// Declare the padding the caller applies around the textarea (for example
    /// via `.p_1()`). The hint is inset by this padding plus the textarea's own
    /// text insets so it starts exactly where typed text and the caret do.
    pub(crate) fn outer_padding(mut self, padding: Pixels) -> Self {
        self.outer_padding = padding;
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
        let size = Size::default();
        let left = self.outer_padding + size.input_px();
        let right = self.outer_padding + size.input_px();
        let top = self.outer_padding + size.input_py();
        // The textarea lays text out with a fixed 1.25rem line height; match it
        // so the hint shares the first line's baseline with typed text.
        let metrics = (
            input_text_size(size, window),
            rems(1.25).to_pixels(window.rem_size()),
        );
        render_hinted_control(
            self.textarea,
            hint,
            is_empty,
            is_focused,
            size,
            Some(left),
            Some(right),
            Some(top),
            Some(top),
            false,
            false,
            false,
            Some(metrics),
            None,
            None,
        )
    }
}

#[derive(IntoElement)]
pub(crate) struct HintedEditor {
    editor: Editor,
    state: Entity<EditorState>,
    outer_padding: Pixels,
    container_h_full: bool,
}

impl HintedEditor {
    pub(crate) fn new(state: &Entity<EditorState>) -> Self {
        Self {
            editor: Editor::new(state).bordered(false).border_0(),
            state: state.clone(),
            outer_padding: px(0.0),
            container_h_full: false,
        }
    }

    pub(crate) fn appearance(mut self, appearance: bool) -> Self {
        self.editor = self.editor.appearance(appearance);
        self
    }

    /// Declare the padding the caller applies around the editor (for example
    /// via `.p_3()`). The hint is inset by this padding plus the editor's own
    /// text insets so it lines up with typed text and the caret.
    pub(crate) fn outer_padding(mut self, padding: Pixels) -> Self {
        self.outer_padding = padding;
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
        let value = self.state.read(cx).value();
        let is_empty = value.is_empty();
        let is_focused = self.state.focus_handle(cx).is_focused(window);
        let size = Size::default();
        // The editor renders code in the theme's mono font at its own size and
        // line height, which is what its caret and typed text follow.
        let font_size = cx.theme().mono_font_size;
        let font_family = cx.theme().mono_font_family.clone();
        // Mirrors the editor's internal paddings: code editors use a narrower
        // left inset than plain inputs, then indent the text past the gutter.
        let gutter = match hint.as_ref().map(|hint| hint.layout) {
            Some(InputHintLayout::CodeEditor { folding }) => code_editor_gutter_width(
                // `Rope::lines_len` counts lines the same way: a trailing
                // newline opens another, empty line.
                value.split('\n').count(),
                font_size,
                &font_family,
                folding,
                window,
            ),
            _ => px(0.0),
        };
        let left = self.outer_padding + size.input_px().min(px(6.0)) + gutter;
        let right = self.outer_padding + size.input_px();
        let top = self.outer_padding + size.input_py();
        render_hinted_control(
            self.editor,
            hint,
            is_empty,
            is_focused,
            size,
            Some(left),
            Some(right),
            Some(top),
            Some(top),
            false,
            false,
            self.container_h_full,
            // gpui-component's `EDITOR_LINE_HEIGHT`.
            Some((font_size, (font_size * 1.5).round())),
            Some(font_family),
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::input::EditorState;
    use gpui_kit::{AppContext as _, Bounds, Context, Render, TestAppContext};

    /// Asserts the drawn hint starts on the same line and column as the caret.
    fn assert_hint_matches_caret(cx: &mut gpui_kit::VisualTestContext, caret: Bounds<Pixels>) {
        let hint = cx.debug_bounds("input-hint").expect("hint must render");
        let left = f32::from(hint.left() - caret.left()).abs();
        assert!(left < 0.5, "hint starts {left}px from the caret");
        let caret_center = caret.top() + caret.size.height / 2.0;
        let hint_center = hint.top() + hint.size.height / 2.0;
        let top = f32::from(caret_center - hint_center).abs();
        assert!(top < 1.5, "hint line is {top}px off the caret line");
    }

    struct EditorProbe {
        state: Entity<EditorState>,
    }

    impl Render for EditorProbe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(px(320.0))
                    .h(px(120.0))
                    .child(
                        HintedEditor::new(&self.state)
                            .size_full()
                            .appearance(false)
                            .p_3()
                            .outer_padding(px(12.0))
                            .container_h_full(),
                    ),
            )
        }
    }

    #[gpui_kit::test]
    fn editor_hint_starts_at_the_caret(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut state = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            // The startup-command and snippet editors disable folding.
            let editor = cx.new(|cx| EditorState::new(window, cx).folding(false));
            register_code_editor_input_hint(&editor, "hint", false, cx);
            state = Some(editor.clone());
            EditorProbe { state: editor }
        });
        let state = state.unwrap();
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.focus(&state.read(cx).focus_handle(cx), cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx).clear(cx);
        });

        let caret = state.read_with(cx, |state, _| {
            let geometry = state.prepaint_caret_geometry().expect("prepaint must run");
            geometry.cursor_bounds()
        });
        assert_hint_matches_caret(cx, caret);
    }

    struct TextareaProbe {
        state: Entity<TextareaState>,
    }

    impl Render for TextareaProbe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                div().absolute().top_0().left_0().w(px(320.0)).child(
                    HintedTextarea::new(&self.state)
                        .appearance(false)
                        .p_1()
                        .outer_padding(px(4.0)),
                ),
            )
        }
    }

    #[gpui_kit::test]
    fn textarea_hint_starts_at_the_caret(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut state = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let textarea = cx.new(|cx| TextareaState::new(window, cx));
            register_textarea_hint(&textarea, "hint", cx);
            state = Some(textarea.clone());
            TextareaProbe { state: textarea }
        });
        let state = state.unwrap();
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.focus(&state.read(cx).focus_handle(cx), cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx).clear(cx);
        });

        let caret = state.read_with(cx, |state, _| {
            let geometry = state.prepaint_caret_geometry().expect("prepaint must run");
            geometry.cursor_bounds()
        });
        assert_hint_matches_caret(cx, caret);
    }

    struct InputProbe {
        state: Entity<InputState>,
    }

    impl Render for InputProbe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(px(320.0))
                    .child(HintedInput::new(&self.state).large()),
            )
        }
    }

    #[gpui_kit::test]
    fn input_hint_starts_at_the_caret(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut state = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx));
            register_input_hint(&input, "hint", cx);
            state = Some(input.clone());
            InputProbe { state: input }
        });
        let state = state.unwrap();
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.focus(&state.read(cx).focus_handle(cx), cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx).clear(cx);
        });

        let caret = state.read_with(cx, |state, _| {
            let geometry = state.prepaint_caret_geometry().expect("prepaint must run");
            geometry.cursor_bounds()
        });
        assert_hint_matches_caret(cx, caret);
    }
}
