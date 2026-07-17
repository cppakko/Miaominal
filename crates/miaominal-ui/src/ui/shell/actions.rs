#[path = "actions/keychain.rs"]
mod keychain;
#[path = "actions/notification.rs"]
mod notification;
#[path = "actions/settings.rs"]
mod settings;
pub(in crate::ui::shell) use notification::{
    ValidationFailure, ValidationNotificationKind, error_notification, success_notification,
    validation_notification, warning_notification,
};
pub(in crate::ui::shell) use settings::{
    ai_provider_kind_chat_supported, ai_provider_kind_label_key, ai_provider_select_options,
    web_search_endpoint_placeholder, web_search_provider_kind_label_key,
};
