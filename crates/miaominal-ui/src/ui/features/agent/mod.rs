//! Agent feature: retained state, views and workflow under one owner.
use crate::ui::components::overlay_target::OverlayTarget;

use crate::ui::assets::AppIcon;
use crate::ui::components::animation::{
    OVERLAY_ENTER_DURATION, TerminalSearchAnimation, WorkspaceSidePanelTransition,
    container_transition_animation, list_enter_animation, overlay_enter_animation,
    render_basic_dialog, short_feedback_animation,
};
use crate::ui::components::forms::{
    new_input_state, set_input_placeholder, set_input_value, set_textarea_placeholder,
};
use crate::ui::components::notifications::{
    error_notification, success_notification, warning_notification,
};
use crate::ui::components::tokens::color_with_alpha;
use crate::ui::components::{
    BasicDialogActionTone, HintedInput, HintedTextarea, SearchInputStyle, TextInputSurface,
    basic_dialog_action_button, card_surface, search_filter_input, surface_text_input_stack,
};
use crate::ui::contracts::commands::{AgentDeferredCommand, AppCommand, DeferredAppCommand};
use crate::ui::contracts::preferences::{AgentPreferencesPort, AgentPreferencesSnapshot};
use crate::ui::contracts::security::LocalVaultStatus;
use crate::ui::contracts::session::{
    SessionQueryPort, SessionTerminalPort, SessionTerminalTarget, TerminalLease,
    TerminalLeaseError, TerminalLeaseGrant,
};
use crate::ui::contracts::workspace::TabId;
use crate::ui::utils::{format_local_timestamp, truncate_with_ellipsis};
use gpui_kit::component::{
    Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{ContextMenuExt as _, PopupMenuItem},
    scroll::ScrollableElement,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, CursorStyle, ElementId, Entity, ExternalPaths, Focusable, FontWeight,
    InteractiveElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point,
    Render, SharedString, Styled, Window, canvas, div, px, rgb,
};
use miaominal_core::profile::SessionProfile;
use std::time::{Duration, SystemTime};

mod controller;
use controller::*;
pub(in crate::ui) use controller::{AgentController, AgentControllerArgs, PromptHistoryDirection};
mod conversation_view;
mod views;
const AGENT_PANEL_DEFAULT_WIDTH: f32 = 356.0;
pub(in crate::ui) use views::session_agent_panel::clamp_session_agent_panel_width;
