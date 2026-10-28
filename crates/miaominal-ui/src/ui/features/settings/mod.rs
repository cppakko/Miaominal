//! Settings feature: retained state, views and workflow under one owner.
use crate::ui::components::overlay_target::OverlayTarget;

use crate::ui::assets::AppIcon;
use crate::ui::components::animation::{
    CONTAINER_TRANSITION_DURATION, render_basic_dialog, render_bottom_popup,
};
use crate::ui::components::forms::{
    localized_secret_placeholder, new_input_state, set_input_placeholder, set_input_value,
};
use crate::ui::components::notifications::{
    AppNotification, AppNotificationPriority, AppNotificationTone, ValidationFailure,
    ValidationNotificationKind, error_notification, success_notification, validation_notification,
    warning_notification,
};
use crate::ui::components::selection::SelectOption;
use crate::ui::components::tokens::color_with_alpha;
use crate::ui::components::{
    BasicDialogActionTone, TextInputSurface, basic_dialog_action_button, bottom_popup_panel,
    field_label, icon_button, md3_select, pill_label, setting_field_with_reset_action,
    surface_secret_text_input_stack, surface_text_input, surface_text_input_stack,
};
use crate::ui::contracts::auto_sync::AutoSyncControlPort;
use crate::ui::contracts::catalog::CatalogQueryPort;
use crate::ui::contracts::commands::{AppCommand, DeferredAppCommand, SettingsDeferredCommand};
use crate::ui::contracts::navigation::{SettingsDestination, SidebarSection};
use crate::ui::contracts::preferences::{SftpBrowserSide, ai_provider_kind_chat_supported};
use crate::ui::contracts::proxies::ProxyCommitPort;
use crate::ui::contracts::security::{
    LocalVaultPassphrasePopupMode, LocalVaultStatus, SecretRevealTarget,
};
use crate::ui::contracts::settings::{
    AiProviderSaveDraft, LocalVaultActionRequest, ProxySaveDraft, SecretRevealRequest,
    SettingsPersistencePort, SyncPassphraseSaveDraft, SyncProviderConfigSaveDraft,
    WebSearchSaveDraft,
};
use crate::ui::i18n;
use crate::ui::utils::format_local_timestamp;
use gpui_kit::component::{
    Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerState},
    h_flex,
    input::InputState,
    scroll::ScrollableElement,
    select::SelectState,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, Context, Entity, Focusable, FontWeight, InteractiveElement,
    MouseButton, MouseDownEvent, MouseUpEvent, Render, SharedString, Styled, WeakEntity, Window,
    WindowControlArea, div, px, rgb,
};
use miaominal_settings::{AiProviderKind, WebSearchProviderKind};
use miaominal_sync::SyncStatus;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

mod controller;
use controller::*;
pub(in crate::ui) use controller::{
    LocalVaultChangePassphraseResult, LocalVaultEnableResult, LocalVaultOperationResult,
    LocalVaultUnlockResult, SettingsController, SettingsControllerArgs,
};
mod bridge_security_notification;
mod labels;
mod onboarding;
mod page;
mod search;
pub(in crate::ui) use search::search_entries;

pub(in crate::ui) fn search_destination(id: &'static str) -> Option<SettingsDestination> {
    search::search_destination(id).map(|_| SettingsDestination::SearchItem(id))
}
mod preferences;
mod provider_labels;
mod sync_status;
use bridge_security_notification::*;
use labels::*;
pub(in crate::ui) use onboarding::render_onboarding_page;
pub(in crate::ui) use page::render_settings_page;
use provider_labels::*;
pub(in crate::ui) use sync_status::sync_status_summary;
