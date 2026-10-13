//! Sftp feature: retained state, views and workflow under one owner.

use crate::ui::assets::AppIcon;
use crate::ui::components::animation::render_basic_dialog_with_config;
use crate::ui::components::forms::{new_input_state, set_input_placeholder, set_input_value};
use crate::ui::components::notifications::{
    ValidationNotificationKind, error_notification, success_notification, validation_notification,
};
use crate::ui::components::overlay_target::OverlayTarget;
use crate::ui::components::tokens::color_with_alpha;
use crate::ui::components::{
    BasicDialogActionTone, BasicDialogHeaderAlignment, BasicDialogIcon, HintedInput, badge,
    basic_dialog_action_button, card_surface, icon_button_with_tooltip,
};
use crate::ui::contracts::commands::AppCommand;
use crate::ui::contracts::navigation::SidebarSection;
use crate::ui::contracts::preferences::SftpBrowserSide;
use crate::ui::contracts::session::SessionQueryPort;
use crate::ui::contracts::workspace::TabId;
use crate::ui::i18n;
use crate::ui::utils::{format_byte_size, format_local_timestamp, truncate_with_ellipsis};
use gpui_kit::component::{
    Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::InputState,
    menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
    scroll::ScrollableElement,
    table::{Column, TableDelegate, TableEvent, TableState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, ClickEvent, Context, Div, ElementId, Entity, ExternalPaths, Focusable,
    FontWeight, InteractiveElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Render, ScrollWheelEvent, SharedString, Stateful, Styled, WeakEntity, Window,
    div, px, rgb,
};
use miaominal_core::profile::SessionProfile;
use miaominal_sftp::{SftpEntry, TransferDirection, TransferId};

mod controller;
use controller::*;
pub(in crate::ui) use controller::{SftpController, SftpControllerArgs};
mod browser;
mod page;
mod system_file_icons;
use browser::*;
use system_file_icons::*;
