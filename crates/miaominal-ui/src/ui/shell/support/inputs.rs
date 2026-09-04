use super::super::*;
use crate::ui::components::{
    register_code_editor_input_hint, register_input_hint, register_textarea_hint,
};

pub(in crate::ui::shell) fn localized_secret_placeholder(
    has_saved: bool,
    fallback_key: &'static str,
) -> String {
    if has_saved {
        crate::ui::i18n::string("placeholders.saved.keep_existing")
    } else {
        crate::ui::i18n::string(fallback_key)
    }
}

pub(in crate::ui::shell) fn new_input_state<T: 'static>(
    placeholder: impl Into<SharedString>,
    default_value: impl Into<SharedString>,
    masked: bool,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    let default_value = default_value.into();

    let input = cx.new(move |cx| {
        let input = InputState::new(window, cx)
            .placeholder("")
            .default_value(default_value);

        if masked { input.masked(true) } else { input }
    });
    register_input_hint(&input, placeholder, cx);
    input
}

pub(in crate::ui::shell) fn set_input_value(
    input: &Entity<InputState>,
    value: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let value = value.into();
    input.update(cx, move |input, cx| input.set_value(value, window, cx));
}

pub(in crate::ui::shell) fn set_textarea_value(
    input: &Entity<TextareaState>,
    value: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let value = value.into();
    input.update(cx, move |input, cx| input.set_value(value, window, cx));
}

pub(in crate::ui::shell) fn set_editor_value(
    input: &Entity<EditorState>,
    value: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let value = value.into();
    input.update(cx, move |input, cx| input.set_value(value, window, cx));
}

pub(in crate::ui::shell) fn set_input_placeholder(
    input: &Entity<InputState>,
    placeholder: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let placeholder = placeholder.into();
    register_input_hint(input, placeholder, cx);
    input.update(cx, move |input, cx| input.set_placeholder("", window, cx));
}

pub(in crate::ui::shell) fn set_textarea_placeholder(
    input: &Entity<TextareaState>,
    placeholder: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let placeholder = placeholder.into();
    register_textarea_hint(input, placeholder, cx);
    input.update(cx, move |input, cx| input.set_placeholder("", window, cx));
}

pub(in crate::ui::shell) fn set_code_editor_input_placeholder(
    input: &Entity<EditorState>,
    placeholder: impl Into<SharedString>,
    folding: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let placeholder = placeholder.into();
    register_code_editor_input_hint(input, placeholder, folding, cx);
    input.update(cx, move |input, cx| input.set_placeholder("", window, cx));
}

pub(in crate::ui::shell) fn set_input_masked(
    input: &Entity<InputState>,
    masked: bool,
    focus: bool,
    window: &mut Window,
    cx: &mut App,
) {
    input.update(cx, move |input, cx| {
        input.set_masked(masked, window, cx);
        if focus {
            input.focus(window, cx);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::input::{AnyInputState, Editor, Textarea};
    use gpui_kit::{IntoElement, Render, TestAppContext, div};

    struct TypedInputTestView {
        textarea: Entity<TextareaState>,
        editor: Entity<EditorState>,
    }

    impl Render for TypedInputTestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .child(Textarea::new(&self.textarea))
                .child(Editor::new(&self.editor))
        }
    }

    #[gpui_kit::test]
    fn textarea_and_editor_keep_distinct_state_and_value_lifecycles(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let textarea = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(3, 8)
                    .default_value("first\nsecond")
            });
            let editor = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("bash")
                    .folding(false)
                    .default_value("echo ready")
            });
            TypedInputTestView { textarea, editor }
        });

        let (textarea, editor) =
            view.read_with(cx, |view, _| (view.textarea.clone(), view.editor.clone()));
        assert!(textarea.read_with(cx, |state, _| state.is_multi_line()));
        assert!(editor.read_with(cx, |state, _| state.is_code_editor()));
        assert_eq!(
            textarea.read_with(cx, |state, _| state.value()),
            "first\nsecond"
        );
        assert_eq!(editor.read_with(cx, |state, _| state.value()), "echo ready");
        assert_eq!(
            AnyInputState::from(textarea.clone()).as_textarea(),
            Some(&textarea)
        );
        assert_eq!(
            AnyInputState::from(editor.clone()).as_editor(),
            Some(&editor)
        );

        cx.update(|window, cx| {
            set_textarea_value(&textarea, "updated\ntext", window, cx);
            set_editor_value(&editor, "", window, cx);
        });
        assert_eq!(
            textarea.read_with(cx, |state, _| state.value()),
            "updated\ntext"
        );
        assert!(editor.read_with(cx, |state, _| state.value().is_empty()));
    }
}
