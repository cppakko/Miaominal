use super::super::state::{
    PendingLocalDataResetConfirmState, PendingLocalDataResetConfirmationPopupState,
    PendingSyncPassphraseClearConfirmPopupState, PendingSyncPassphrasePopupState,
    SyncPassphraseOperation, SyncSecretSaveOperation,
};
use super::super::support::set_input_masked;
use super::super::*;
use crate::ui::i18n;
use gpui_component::WindowExt;
use miaominal_secrets::{SecretKind, SecretStore};
use miaominal_services::{
    LocalVaultMode, LocalVaultPassphraseChangeOutcome, LocalVaultTransition, SettingsService,
};
use miaominal_settings::{
    self, AiProviderConfig, AiProviderKind, AppLanguage, KeyBinding, LastTabCloseBehavior,
    LocalVaultAutoLockDuration, MonitorHistoryDuration, TerminalRightClickBehavior, ThemeId,
    WebSearchConfig, WebSearchProviderKind,
};
use miaominal_sync::engine::SyncEngine;
use miaominal_sync::{SyncConfig, SyncProvider, SyncStatus};

const LOCAL_DATA_RESET_CONFIRMATION_TOKEN: &str = "RESET";

#[path = "settings/ai_providers.rs"]
mod ai_providers;
#[path = "settings/appearance.rs"]
mod appearance;
#[path = "settings/key_bindings.rs"]
mod key_bindings;
#[path = "settings/labels.rs"]
mod labels;
#[path = "settings/local_data_reset.rs"]
mod local_data_reset;
#[path = "settings/local_vault.rs"]
mod local_vault;
#[path = "settings/misc.rs"]
mod misc;
#[path = "settings/secret_visibility.rs"]
mod secret_visibility;
#[path = "settings/sync.rs"]
mod sync;
#[path = "settings/web_search.rs"]
mod web_search;

pub(in crate::ui::shell) use labels::{
    ai_provider_kind_chat_supported, ai_provider_kind_label_key, ai_provider_select_options,
    web_search_endpoint_placeholder, web_search_provider_kind_label_key,
};
