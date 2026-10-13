use crate::ui::assets::AppIcon;
use gpui_kit::component::Icon;
#[cfg(test)]
use gpui_kit::component::Root;
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, InputState},
    menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, Bounds, ClipboardItem, Context, Entity, FocusHandle, InteractiveElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render,
    ScrollDelta, ScrollHandle, ScrollWheelEvent, SharedString, Styled, Subscription, Window, div,
    prelude::*, px, rgb,
};
use miaominal_core::profile::{AuthMethod, SessionProfile};
use miaominal_settings;
use miaominal_sync::SyncProvider;
use miaominal_terminal::{
    TerminalState, terminal_cell_width_default, terminal_line_height_default,
};
use tokio::runtime::Handle as TokioHandle;

mod actions;
#[path = "shell/app_view.rs"]
mod app_view;
mod application_sync;
mod bootstrap;
mod bootstrap_subscriptions;

mod containers;
pub(in crate::ui) mod controllers;
use crate::ui::features::forwarding;
use crate::ui::features::hosts;
use crate::ui::features::snippets;
use crate::ui::features::trusted_hosts;
use hosts::{HostsController, HostsControllerArgs};
mod forms;
mod layout;
use crate::ui::components::metrics;
mod navigation;
use crate::ui::components::notifications as notification_center;
mod overlays;
mod pages;
mod panes;
mod render;
mod session;

pub(in crate::ui) mod state;
mod support;

mod tab_transfer;
mod terminal;
use crate::ui::workspace::model as workspace;

use app_view::ShellUiState;
pub use app_view::{AppView, AppWindowRole};

pub(crate) use crate::ui::components::{
    HintedInput, card_surface, fab_button, icon_button_with_tooltip,
};
pub(in crate::ui::shell) use actions::{error_notification, success_notification};

use containers::{AppViewSubscriptions, RootSubscriptions};
use controllers::ControllerSet;
pub(in crate::ui::shell) use controllers::{
    AgentControllerArgs, AgentDeferredCommand, AppCommand, ClosedSessionTabState,
    DeferredAppCommand, KeychainControllerArgs, LocalVaultActionRequest,
    LocalVaultChangePassphraseResult, LocalVaultEnableResult, LocalVaultOperationResult,
    LocalVaultRootExt, LocalVaultUnlockResult, PromptHistoryDirection, SessionController,
    SessionControllerArgs, SessionDeferredCommand, SessionEventOutcome, SessionEventTabRemoval,
    SessionNotificationTone, SessionPortSnapshot, SessionPurpose, SessionSidePanelView,
    SettingsController, SettingsControllerArgs, SettingsDeferredCommand, SftpControllerArgs,
    SftpDeferredCommand, TerminalMenuCommand, TransferredSessionTab, TransferredSessionWorkspaceUi,
    TransferredSftpTab, TransferredSftpWindowUi,
};
use forms::WorkspaceForms;
pub(in crate::ui::shell) use layout::{ChromeAppViewExt, WorkspacePanesAppViewExt};
pub(in crate::ui::shell) use metrics::*;
pub(in crate::ui::shell) use navigation::SidebarSection;
pub(crate) use notification_center::publish_app_notification;
pub(crate) use notification_center::{
    AppNotificationAction, initialize_notification_center, push_app_notification,
    render_notification_center_popover, show_pending_app_notifications,
};
pub(in crate::ui::shell) use pages::SettingsDestination;
pub(in crate::ui::shell) use panes::PaneId;
use panes::{PaneCloseAnimation, PaneSplitAnimation, PaneSplitAnimationKind, ParkedPane};

pub(in crate::ui::shell) use state::{DraggedTab, ShellState};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub(in crate::ui::shell) use crate::ui::features::session::WorkspaceTerminalInputExt;
use workspace::{
    ClosePlanStep, ClosedSftpTabState, ClosedTabBundle, PaneLayout, SplitAxis, SplitDirection,
    TabDescriptor, TabId, TabKindTag, TabPlacement, TabRegistry, TabState, TabWorkspaceState,
    WorkspaceModel, remember_page_editor_sidebar, remember_sidebar_section, reopened_tab_id,
    restored_page_editor_sidebar, restored_sidebar_section,
};

pub(in crate::ui::shell) use crate::ui::features::session::{
    render_terminal_canvas_for_pane, terminal_cell_width, terminal_ime_cursor_bounds,
    terminal_line_height,
};
use crate::ui::workspace::transfer::WorkspaceTransferExt;
use support::{OVERLAY_ENTER_DURATION, new_input_state, set_input_placeholder, set_input_value};

pub(in crate::ui::shell) fn color_with_alpha(color: u32, alpha: u8) -> gpui_kit::Rgba {
    gpui_kit::rgba(((color & 0x00ff_ffff) << 8) | alpha as u32)
}

/// Switches the active window's sidebar to the Settings section.
///
/// Invoked from the macOS application menu (Preferences, Cmd+,). Safely does
/// nothing when there is no active window or the active window's root view is
/// not an `AppView` (for example a detached window with a different root).
pub fn open_settings_from_menu(cx: &mut App) {
    let Some(window) = cx.active_window() else {
        return;
    };
    let _ = window.update(cx, |root, window, cx| {
        if let Ok(view) = root.downcast::<AppView>() {
            view.update(cx, |view, cx| {
                view.set_sidebar_section(SidebarSection::Settings, cx);
                view.sync_window_overlays(window, cx);
            });
        }
    });
}

const APP_TITLE: &str = "Miaominal";
pub(in crate::ui::shell) const SESSION_MONITOR_PANEL_WIDTH: f32 = 356.0;
pub(in crate::ui::shell) const SESSION_SFTP_PROGRESS_DEFAULT_FLEX: f32 = 0.26;
use crate::ui::contracts::navigation::PageEditorSidebarKind;
use crate::ui::workspace::presentation::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum PageEditorSidebarTransitionPhase {
    Entering,
    Exiting,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct PageEditorSidebarTransition {
    pub(in crate::ui::shell) kind: PageEditorSidebarKind,
    pub(in crate::ui::shell) phase: PageEditorSidebarTransitionPhase,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

use crate::ui::components::animation::{
    WorkspaceSidePanelTransition, WorkspaceSidePanelTransitionPhase,
};
use crate::ui::components::selection::ManagedKeySelectItem;
use crate::ui::contracts::security::{LocalVaultPassphrasePopupMode, LocalVaultStatus};
use trusted_hosts::TrustedHostsController;

// Phase 5: remove these logical Shell seams with the shared presentation imports.
use snippets::SnippetsController;

use forwarding::{ForwardingController, ForwardingControllerArgs};
