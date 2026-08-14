use super::super::*;
use crate::ui::i18n;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum ValidationNotificationKind {
    RequiredInputMissing,
    InvalidInput,
}

impl ValidationNotificationKind {
    fn title(self) -> String {
        i18n::string(match self {
            Self::RequiredInputMissing => "notifications.validation.required_input_missing",
            Self::InvalidInput => "notifications.validation.invalid_input",
        })
    }

    fn tone(self) -> AppNotificationTone {
        match self {
            Self::RequiredInputMissing => AppNotificationTone::Warning,
            Self::InvalidInput => AppNotificationTone::Error,
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct ValidationFailure {
    pub(in crate::ui::shell) kind: ValidationNotificationKind,
    pub(in crate::ui::shell) message: String,
}

impl ValidationFailure {
    pub(in crate::ui::shell) fn required(message: impl Into<String>) -> Self {
        Self {
            kind: ValidationNotificationKind::RequiredInputMissing,
            message: message.into(),
        }
    }

    pub(in crate::ui::shell) fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: ValidationNotificationKind::InvalidInput,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ValidationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ValidationFailure {}

pub(in crate::ui::shell) fn error_notification(
    title: impl Into<String>,
    message: impl Into<String>,
) -> AppNotification {
    AppNotification::new(
        AppNotificationTone::Error,
        AppNotificationPriority::Normal,
        title,
        message,
    )
}

pub(in crate::ui::shell) fn success_notification(
    title: impl Into<String>,
    message: impl Into<String>,
) -> AppNotification {
    AppNotification::new(
        AppNotificationTone::Success,
        AppNotificationPriority::Low,
        title,
        message,
    )
}

pub(in crate::ui::shell) fn warning_notification(
    title: impl Into<String>,
    message: impl Into<String>,
) -> AppNotification {
    AppNotification::new(
        AppNotificationTone::Warning,
        AppNotificationPriority::Normal,
        title,
        message,
    )
}

pub(in crate::ui::shell) fn validation_notification(
    kind: ValidationNotificationKind,
    message: String,
) -> AppNotification {
    let notification_id = format!("validation-error-{message}");
    AppNotification::new(
        kind.tone(),
        AppNotificationPriority::Normal,
        kind.title(),
        message,
    )
    .id1::<AppView>(notification_id)
}
