use super::super::*;
use crate::ui::i18n;
use gpui_component::{WindowExt as _, notification::Notification};

const NOTIFICATION_CORNER_RADIUS: f32 = 20.0;
const NOTIFICATION_ICON_SIZE: f32 = 28.0;
const NOTIFICATION_ICON_CONTAINER_SIZE: f32 = 44.0;
const NOTIFICATION_CLOSE_BUTTON_SIZE: f32 = 30.0;
const NOTIFICATION_CLOSE_BUTTON_RADIUS: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NotificationTone {
    Success,
    Warning,
    Error,
}

impl NotificationTone {
    fn icon(self) -> IconName {
        match self {
            Self::Success => IconName::CircleCheck,
            Self::Warning => IconName::TriangleAlert,
            Self::Error => IconName::CircleX,
        }
    }
}

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

    fn tone(self) -> NotificationTone {
        match self {
            Self::RequiredInputMissing => NotificationTone::Warning,
            Self::InvalidInput => NotificationTone::Error,
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

impl AppView {
    pub(in crate::ui::shell) fn style_notification(notification: Notification) -> Notification {
        let roles = settings::current_theme().material.roles;

        notification
            .border_0()
            .bg(rgb(roles.surface_container_highest))
            .rounded(px(NOTIFICATION_CORNER_RADIUS))
    }

    fn custom_notification(
        tone: NotificationTone,
        title: impl Into<SharedString>,
        message: impl Into<SharedString>,
    ) -> Notification {
        let title = title.into();
        let message = message.into();
        let material = settings::current_theme().material;
        let roles = material.roles;
        let accent = match tone {
            NotificationTone::Success => material.extended.success.color,
            NotificationTone::Warning => material.extended.warning.color,
            NotificationTone::Error => roles.error,
        };
        let icon = tone.icon();

        Self::style_notification(Notification::new().content(move |_, _, cx| {
            let dismiss_entity = cx.entity().clone();

            h_flex()
                .w_full()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex_none()
                        .size(px(NOTIFICATION_ICON_CONTAINER_SIZE))
                        .rounded(px(999.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(color_with_alpha(accent, 0x18))
                        .child(
                            Icon::new(icon.clone())
                                .size(px(NOTIFICATION_ICON_SIZE))
                                .text_color(rgb(accent)),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(roles.on_surface))
                                .child(title.clone()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .line_height(settings::scaled_line_height(18.0))
                                .text_color(rgb(roles.on_surface_variant))
                                .child(message.clone()),
                        ),
                )
                .child(div().flex_none().child(icon_button(
                    AppIcon::Close,
                    NOTIFICATION_CLOSE_BUTTON_SIZE,
                    NOTIFICATION_CLOSE_BUTTON_RADIUS,
                    Some(roles.surface_container_low),
                    Some(roles.on_surface_variant),
                    None,
                    move |window, cx| {
                        dismiss_entity.update(cx, |this, cx| {
                            this.dismiss(window, cx);
                        });
                    },
                )))
                .into_any_element()
        }))
    }

    pub(in crate::ui::shell) fn error_notification(
        title: impl Into<SharedString>,
        message: impl Into<SharedString>,
    ) -> Notification {
        Self::custom_notification(NotificationTone::Error, title, message)
    }

    pub(in crate::ui::shell) fn success_notification(
        title: impl Into<SharedString>,
        message: impl Into<SharedString>,
    ) -> Notification {
        Self::custom_notification(NotificationTone::Success, title, message)
    }

    fn validation_notification(kind: ValidationNotificationKind, message: String) -> Notification {
        let notification_id = SharedString::from(format!("validation-error-{message}"));

        Self::custom_notification(kind.tone(), kind.title(), message)
            .id1::<AppView>(notification_id)
    }

    pub(in crate::ui::shell) fn notify_validation_failure(
        &mut self,
        kind: ValidationNotificationKind,
        message: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        let message = message.into();

        self.status_message = message.clone();
        let notification = Self::validation_notification(kind, message);
        self.with_active_window(cx, move |window, cx| {
            window.push_notification(notification, cx);
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn notify_validation_failure_in_window(
        &mut self,
        window: &mut Window,
        kind: ValidationNotificationKind,
        message: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        let message = message.into();

        self.status_message = message.clone();
        window.push_notification(Self::validation_notification(kind, message), cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn with_active_window(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        let Some(window_handle) = cx.active_window() else {
            return;
        };
        let _ = window_handle.update(cx, move |_, window, cx| {
            update(window, cx);
        });
    }
}
