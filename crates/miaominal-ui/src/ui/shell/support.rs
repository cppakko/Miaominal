#[path = "support/animation.rs"]
mod animation;
#[path = "support/custom_glyphs.rs"]
mod custom_glyphs;
#[path = "support/group_accent.rs"]
mod group_accent;
#[path = "support/inputs.rs"]
mod inputs;
#[path = "support/sync_ops.rs"]
mod sync_ops;
#[path = "support/terminal.rs"]
mod terminal;
#[path = "support/terminal_element.rs"]
mod terminal_element;

pub(in crate::ui::shell) use animation::{
    BasicDialogConfig, CONTAINER_TRANSITION_DURATION, LIST_ENTER_DURATION, OVERLAY_ENTER_DURATION,
    container_transition_animation, list_enter_animation, overlay_enter_animation,
    render_basic_dialog, render_basic_dialog_with_config, render_bottom_popup,
    short_feedback_animation,
};
pub(in crate::ui::shell) use group_accent::{GroupAccentPalette, group_accent_palette};
pub(in crate::ui::shell) use inputs::{
    localized_secret_placeholder, new_input_state, set_code_editor_input_placeholder,
    set_input_masked, set_input_placeholder, set_input_value,
};
pub(in crate::ui::shell) use sync_ops::sync_status_summary;
pub(in crate::ui::shell) use terminal::{
    TerminalKeyAction, TerminalKeyEvent, TerminalKeyPhase, classify_terminal_key,
    terminal_cell_width, terminal_line_height,
};
pub(in crate::ui::shell) use terminal_element::{
    TerminalScrollbarMetrics, render_terminal_canvas_for_pane, terminal_scrollbar_metrics,
    terminal_scrollbar_offset_for_pointer,
};
