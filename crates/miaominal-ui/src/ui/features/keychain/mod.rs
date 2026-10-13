//! Keychain feature: retained state, views and workflow under one owner.
use crate::ui::components::metrics::EDITOR_DRAWER_WIDTH;
use crate::ui::components::overlay_target::OverlayTarget;

use crate::ui::assets::AppIcon;
use crate::ui::components::animation::render_basic_dialog;
use crate::ui::components::forms::{
    new_input_state, set_code_editor_input_placeholder, set_editor_value, set_input_placeholder,
    set_input_value,
};
use crate::ui::components::notifications::{
    ValidationNotificationKind, error_notification, success_notification, validation_notification,
};
use crate::ui::components::selection::ProfileSelectItem;
use crate::ui::components::{
    BasicDialogActionTone, SearchInputStyle, TextInputSurface, badge, basic_dialog_action_button,
    card_surface, editor_footer_actions, fab_button, fab_icon_button, field_label, icon_button,
    md3_select, page_primary_icon_tile, page_section_title, search_filter_input,
    surface_text_editor, surface_text_input, surface_text_input_stack,
};
use crate::ui::contracts::catalog::CatalogQueryPort;
use crate::ui::contracts::commands::{
    AppCommand, DeferredAppCommand, KeychainDeferredCommand, ManagedKeysChange,
};
use crate::ui::contracts::keychain::{
    ManagedKeyCommitPort, ManagedKeyDeployRequest, ManagedKeyImportRequest, ManagedKeyQueryPort,
};
use crate::ui::contracts::navigation::PageEditorSidebarKind;
use crate::ui::contracts::security::LocalVaultStatus;
use anyhow::{Result, anyhow};
use gpui_kit::component::{
    Icon, IconName, Sizable as _, h_flex,
    input::{EditorState, InputEvent, InputState, TabSize},
    menu::PopupMenuItem,
    scroll::ScrollableElement,
    select::{SearchableVec, SelectItem, SelectState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Div, Entity, Focusable, InteractiveElement, SharedString, Styled,
    Subscription, Window, div, px, rgb,
};
use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::profile::{AuthMethod, SessionProfile};
use tokio::runtime::Handle as TokioHandle;

mod controller;
use controller::*;
pub(in crate::ui) use controller::{KeychainController, KeychainControllerArgs};
mod page;

const KEYCHAIN_DEPLOY_DEFAULT_LOCATION: &str = ".ssh";
const KEYCHAIN_DEPLOY_DEFAULT_FILENAME: &str = "authorized_keys";
const KEYCHAIN_DEPLOY_DEFAULT_COMMAND: &str = "if test ! -e $1;\nthen mkdir -p $1;\nchmod 700 $1;\nfi;\nif test ! -e \"$1/$2\";\nthen touch \"$1/$2\";\nchmod 600 \"$1/$2\";\nfi;\necho $3 >> \"$1/$2\";";
