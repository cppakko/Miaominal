#[path = "support/terminal_element.rs"]
mod terminal_element;

pub(in crate::ui::shell) use crate::ui::components::animation::{
    CONTAINER_TRANSITION_DURATION, OVERLAY_ENTER_DURATION,
};
pub(in crate::ui::shell) use crate::ui::components::forms::{
    new_input_state, set_input_placeholder, set_input_value,
};
