use super::*;
use crate::ui::assets::AppIcon;
use crate::ui::i18n;
use gpui::{Anchor, Animation, AnimationExt as _, Global, RenderOnce, ease_in_out, ease_out_quint};
use gpui_component::{
    Selectable, WindowExt as _,
    button::Button,
    notification::Notification,
    popover::{Popover, PopoverState},
};
use std::any::type_name;
use std::rc::Rc;

const NOTIFICATION_HISTORY_LIMIT: usize = 100;
const NOTIFICATION_CORNER_RADIUS: f32 = 20.0;
const NOTIFICATION_ICON_SIZE: f32 = 28.0;
const NOTIFICATION_ICON_CONTAINER_SIZE: f32 = 44.0;
const NOTIFICATION_CLOSE_BUTTON_SIZE: f32 = 30.0;
const NOTIFICATION_CLOSE_BUTTON_RADIUS: f32 = 10.0;
const NOTIFICATION_PANEL_ENTER_DURATION: Duration = Duration::from_millis(200);
const NOTIFICATION_ITEM_ENTER_DURATION: Duration = Duration::from_millis(160);
const NOTIFICATION_ITEM_EXIT_DURATION: Duration = Duration::from_millis(180);
const NOTIFICATION_PANEL_ENTER_OFFSET: f32 = 10.0;
const NOTIFICATION_ITEM_ENTER_OFFSET: f32 = 6.0;
const NOTIFICATION_ITEM_EXIT_OFFSET: f32 = 18.0;

type ToastAction = dyn Fn(&mut Window, &mut App);

fn notification_panel_enter_animation() -> Animation {
    Animation::new(NOTIFICATION_PANEL_ENTER_DURATION).with_easing(ease_out_quint())
}

fn notification_item_enter_animation() -> Animation {
    Animation::new(NOTIFICATION_ITEM_ENTER_DURATION).with_easing(ease_out_quint())
}

fn notification_item_exit_animation() -> Animation {
    Animation::new(NOTIFICATION_ITEM_EXIT_DURATION).with_easing(ease_in_out)
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AppNotificationPriority {
    Low,
    Normal,
    High,
    Critical,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppNotificationTone {
    Info,
    Success,
    Warning,
    Error,
}

impl AppNotificationTone {
    fn icon(self) -> IconName {
        match self {
            Self::Info => IconName::Info,
            Self::Success => IconName::CircleCheck,
            Self::Warning => IconName::TriangleAlert,
            Self::Error => IconName::CircleX,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppNotificationAction {
    OpenSyncSettings,
}

#[derive(Clone)]
pub(crate) struct AppNotificationSpec {
    stable_id: Option<String>,
    pub title: String,
    pub message: String,
    pub tone: AppNotificationTone,
    pub priority: AppNotificationPriority,
    pub action: Option<AppNotificationAction>,
    pub action_label: Option<String>,
    toast_action: Option<Rc<ToastAction>>,
}

impl AppNotificationSpec {
    pub(crate) fn new(
        tone: AppNotificationTone,
        priority: AppNotificationPriority,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            stable_id: None,
            title: title.into(),
            message: message.into(),
            tone,
            priority,
            action: None,
            action_label: None,
            toast_action: None,
        }
    }

    pub(crate) fn stable_id(mut self, id: impl Into<String>) -> Self {
        self.stable_id = Some(id.into());
        self
    }

    pub(in crate::ui::shell) fn id1<T: 'static>(self, key: impl Into<String>) -> Self {
        self.stable_id(format!("{}:{}", type_name::<T>(), key.into()))
    }

    pub(crate) fn structured_action(
        mut self,
        action: AppNotificationAction,
        label: impl Into<String>,
    ) -> Self {
        self.action = Some(action);
        self.action_label = Some(label.into());
        self
    }

    pub(in crate::ui::shell) fn toast_action(
        mut self,
        label: impl Into<String>,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.action_label = Some(label.into());
        self.toast_action = Some(Rc::new(action));
        self
    }

    fn history_entry(&self, id: String, sequence: u64) -> AppNotificationEntry {
        AppNotificationEntry {
            id,
            title: self.title.clone(),
            message: self.message.clone(),
            tone: self.tone,
            priority: self.priority,
            created_at: SystemTime::now(),
            read: false,
            action: self.action,
            action_label: self.action_label.clone().filter(|_| self.action.is_some()),
            sequence,
            dismissal_token: None,
        }
    }

    fn into_toast(self) -> Notification {
        let id = self.stable_id.clone();
        let tone = self.tone;
        let title = SharedString::from(self.title);
        let message = SharedString::from(self.message);
        let action_label = self.action_label.map(SharedString::from);
        let structured_action = self.action;
        let toast_action = self.toast_action;
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let accent = notification_accent(tone, &material);
        let icon = tone.icon();

        let notification = Notification::new().content(move |_, _, cx| {
            let dismiss_entity = cx.entity().clone();
            let dismiss_after_action = dismiss_entity.clone();
            let toast_action = toast_action.clone();
            let has_action = toast_action.is_some() || structured_action.is_some();

            v_flex()
                .w_full()
                .min_w(px(0.0))
                .gap_3()
                .child(
                    h_flex()
                        .w_full()
                        .min_w(px(0.0))
                        .items_center()
                        .gap_3()
                        .child(notification_icon(icon.clone(), accent))
                        .child(notification_text(title.clone(), message.clone(), roles))
                        .child(div().flex_none().child(icon_button(
                            AppIcon::Close,
                            NOTIFICATION_CLOSE_BUTTON_SIZE,
                            NOTIFICATION_CLOSE_BUTTON_RADIUS,
                            Some(roles.surface_container_low),
                            Some(roles.on_surface_variant),
                            None,
                            move |window, cx| {
                                dismiss_entity.update(cx, |this, cx| this.dismiss(window, cx));
                            },
                        ))),
                )
                .when(has_action, |this| {
                    this.child(
                        h_flex().w_full().justify_end().child(
                            basic_dialog_action_button(
                                "app-notification-toast-action",
                                action_label.clone().unwrap_or_else(|| {
                                    i18n::string("notifications.center.open").into()
                                }),
                                BasicDialogActionTone::Default,
                            )
                            .on_click(move |_, window, cx| {
                                if let Some(action) = toast_action.as_ref() {
                                    action(window, cx);
                                } else if let Some(action) = structured_action {
                                    execute_notification_action(action, window, cx);
                                }
                                dismiss_after_action
                                    .update(cx, |this, cx| this.dismiss(window, cx));
                            }),
                        ),
                    )
                })
                .into_any_element()
        });
        let notification = style_notification(notification);
        match id {
            Some(id) => notification.id1::<NotificationCenterToast>(SharedString::from(id)),
            None => notification,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AppNotificationEntry {
    pub id: String,
    pub title: String,
    pub message: String,
    pub tone: AppNotificationTone,
    pub priority: AppNotificationPriority,
    pub created_at: SystemTime,
    pub read: bool,
    pub action: Option<AppNotificationAction>,
    pub action_label: Option<String>,
    sequence: u64,
    dismissal_token: Option<u64>,
}

pub(crate) type AppNotification = AppNotificationSpec;

#[derive(Default)]
struct NotificationCenterState {
    entries: Vec<AppNotificationEntry>,
    next_sequence: u64,
    next_dismissal_token: u64,
    pending_toasts: Vec<AppNotification>,
}

impl NotificationCenterState {
    fn publish(&mut self, notification: &AppNotification) {
        let id = notification
            .stable_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        let entry = notification.history_entry(id.clone(), self.next_sequence);
        if notification.stable_id.is_some()
            && let Some(existing) = self.entries.iter_mut().find(|entry| entry.id == id)
        {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
        if self.entries.len() > NOTIFICATION_HISTORY_LIMIT
            && let Some(index) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.sequence)
                .map(|(index, _)| index)
        {
            self.entries.remove(index);
        }
    }

    fn sorted_entries(&self) -> Vec<AppNotificationEntry> {
        let mut entries = self.entries.clone();
        entries.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| right.sequence.cmp(&left.sequence))
        });
        entries
    }

    fn unread_count(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.read).count()
    }

    fn highest_unread_priority(&self) -> Option<AppNotificationPriority> {
        self.entries
            .iter()
            .filter(|entry| !entry.read)
            .map(|entry| entry.priority)
            .max()
    }

    fn mark_all_read(&mut self) {
        for entry in &mut self.entries {
            entry.read = true;
        }
    }

    fn begin_removal(&mut self, id: &str) -> Option<(String, u64)> {
        let entry = self.entries.iter_mut().find(|entry| entry.id == id)?;
        if entry.dismissal_token.is_some() {
            return None;
        }
        self.next_dismissal_token = self.next_dismissal_token.wrapping_add(1).max(1);
        entry.dismissal_token = Some(self.next_dismissal_token);
        Some((entry.id.clone(), self.next_dismissal_token))
    }

    fn begin_clear_all(&mut self) -> Vec<(String, u64)> {
        let mut removals = Vec::with_capacity(self.entries.len());
        for entry in &mut self.entries {
            if entry.dismissal_token.is_some() {
                continue;
            }
            self.next_dismissal_token = self.next_dismissal_token.wrapping_add(1).max(1);
            entry.dismissal_token = Some(self.next_dismissal_token);
            removals.push((entry.id.clone(), self.next_dismissal_token));
        }
        removals
    }

    fn finish_removals(&mut self, removals: &[(String, u64)]) -> bool {
        let previous_len = self.entries.len();
        self.entries.retain(|entry| {
            !removals
                .iter()
                .any(|(id, token)| entry.id == *id && entry.dismissal_token == Some(*token))
        });
        self.entries.len() != previous_len
    }

    fn enqueue_pending_toast(&mut self, notification: AppNotification) {
        if let Some(id) = notification.stable_id.as_ref()
            && let Some(existing) = self
                .pending_toasts
                .iter_mut()
                .find(|queued| queued.stable_id.as_ref() == Some(id))
        {
            *existing = notification;
        } else {
            self.pending_toasts.push(notification);
        }
    }

    fn take_pending_toasts(&mut self) -> Vec<AppNotification> {
        std::mem::take(&mut self.pending_toasts)
    }
}

#[derive(Default)]
struct GlobalNotificationCenter(NotificationCenterState);

impl Global for GlobalNotificationCenter {}

struct NotificationCenterToast;

#[derive(IntoElement)]
struct NotificationCenterTrigger {
    selected: bool,
    unread: usize,
    badge_color: u32,
    roles: miaominal_settings::theme::Md3Roles,
    tooltip: SharedString,
}

impl Selectable for NotificationCenterTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for NotificationCenterTrigger {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .id("notification-center-trigger")
            .relative()
            .size(px(24.0))
            .child(
                icon_button_with_tooltip(
                    IconName::Bell,
                    self.tooltip,
                    24.0,
                    8.0,
                    Some(self.roles.surface_container),
                    Some(self.roles.on_surface_variant),
                    Some(self.roles.outline_variant),
                    |_, _| {},
                )
                .id("notification-center-trigger-button")
                .hover(move |this| {
                    this.bg(rgb(self.roles.surface_container_highest))
                        .border_color(rgb(self.roles.primary))
                }),
            )
            .when(self.unread > 0, |this| {
                this.child(
                    compact_badge(
                        if self.unread > 99 {
                            "99+".to_string()
                        } else {
                            self.unread.to_string()
                        },
                        self.badge_color,
                        self.roles.on_error,
                    )
                    .absolute()
                    .top(px(-7.0))
                    .right(px(-10.0))
                    .into_any_element(),
                )
            })
    }
}

pub(crate) fn initialize_notification_center(cx: &mut App) {
    if !cx.has_global::<GlobalNotificationCenter>() {
        cx.set_global(GlobalNotificationCenter::default());
    }
}

fn refresh_app_windows(cx: &mut App) {
    for handle in cx.windows() {
        let _ = handle.update(cx, |_, window, _| window.refresh());
    }
}

fn record_notification(notification: &AppNotification, cx: &mut App) {
    initialize_notification_center(cx);
    cx.global_mut::<GlobalNotificationCenter>()
        .0
        .publish(notification);
    refresh_app_windows(cx);
}

pub(crate) fn push_app_notification(
    window: &mut Window,
    notification: AppNotification,
    cx: &mut App,
) {
    record_notification(&notification, cx);
    window.push_notification(notification.into_toast(), cx);
}

pub(crate) fn publish_app_notification(notification: AppNotification, cx: &mut App) {
    record_notification(&notification, cx);
    if let Some(target) = cx.active_window() {
        let _ = target.update(cx, move |_, window, cx| {
            window.push_notification(notification.into_toast(), cx);
        });
    } else {
        cx.global_mut::<GlobalNotificationCenter>()
            .0
            .enqueue_pending_toast(notification);
    }
}

pub(crate) fn show_pending_app_notifications(window: &mut Window, cx: &mut App) {
    initialize_notification_center(cx);
    let pending = cx
        .global_mut::<GlobalNotificationCenter>()
        .0
        .take_pending_toasts();
    for notification in pending {
        window.push_notification(notification.into_toast(), cx);
    }
}

fn notification_center_entries(cx: &App) -> Vec<AppNotificationEntry> {
    cx.try_global::<GlobalNotificationCenter>()
        .map(|center| center.0.sorted_entries())
        .unwrap_or_default()
}

fn notification_center_unread(cx: &App) -> usize {
    cx.try_global::<GlobalNotificationCenter>()
        .map(|center| center.0.unread_count())
        .unwrap_or_default()
}

fn notification_center_highest_unread(cx: &App) -> Option<AppNotificationPriority> {
    cx.try_global::<GlobalNotificationCenter>()
        .and_then(|center| center.0.highest_unread_priority())
}

fn mark_all_notifications_read(cx: &mut App) {
    if cx.has_global::<GlobalNotificationCenter>() {
        let center = cx.global_mut::<GlobalNotificationCenter>();
        center.0.mark_all_read();
        refresh_app_windows(cx);
    }
}

fn clear_all_notifications(cx: &mut App) {
    let removals = if cx.has_global::<GlobalNotificationCenter>() {
        cx.global_mut::<GlobalNotificationCenter>()
            .0
            .begin_clear_all()
    } else {
        Vec::new()
    };
    schedule_notification_removals(removals, cx);
}

fn remove_notification(id: &str, cx: &mut App) {
    let removal = cx
        .has_global::<GlobalNotificationCenter>()
        .then(|| {
            cx.global_mut::<GlobalNotificationCenter>()
                .0
                .begin_removal(id)
        })
        .flatten();
    schedule_notification_removals(removal.into_iter().collect(), cx);
}

fn schedule_notification_removals(removals: Vec<(String, u64)>, cx: &mut App) {
    if removals.is_empty() {
        return;
    }
    refresh_app_windows(cx);
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(NOTIFICATION_ITEM_EXIT_DURATION)
            .await;
        let _ = cx.update(|cx| {
            if cx.has_global::<GlobalNotificationCenter>()
                && cx
                    .global_mut::<GlobalNotificationCenter>()
                    .0
                    .finish_removals(&removals)
            {
                refresh_app_windows(cx);
            }
        });
    })
    .detach();
}

fn execute_notification_action(action: AppNotificationAction, window: &mut Window, cx: &mut App) {
    match action {
        AppNotificationAction::OpenSyncSettings => open_sync_settings_in_main_window(window, cx),
    }
}

fn open_sync_settings_in_main_window(window: &mut Window, cx: &mut App) {
    let Some(main_view) = crate::ui::system_tray::main_window_view(cx) else {
        log::warn!("cannot open sync settings because the main AppView is unavailable");
        return;
    };
    if !crate::ui::system_tray::is_main_window(window, cx) && !crate::ui::restore_main_window(cx) {
        log::warn!("failed to restore the main window before opening sync settings");
    }
    main_view.update(cx, |view, cx| {
        view.navigate_to_settings_destination(SettingsDestination::Sync, cx);
    });
}

fn notification_accent(
    tone: AppNotificationTone,
    material: &miaominal_settings::theme::MaterialTheme,
) -> u32 {
    match tone {
        AppNotificationTone::Info => material.roles.primary,
        AppNotificationTone::Success => material.extended.success.color,
        AppNotificationTone::Warning => material.extended.warning.color,
        AppNotificationTone::Error => material.roles.error,
    }
}

fn notification_priority_label(priority: AppNotificationPriority) -> String {
    let key = match priority {
        AppNotificationPriority::Low => "notifications.center.priority.low",
        AppNotificationPriority::Normal => "notifications.center.priority.normal",
        AppNotificationPriority::High => "notifications.center.priority.high",
        AppNotificationPriority::Critical => "notifications.center.priority.critical",
    };
    i18n::string(key)
}

fn notification_priority_colors(
    priority: AppNotificationPriority,
    material: &miaominal_settings::theme::MaterialTheme,
) -> (u32, u32) {
    match priority {
        AppNotificationPriority::Low => (
            material.roles.secondary_container,
            material.roles.on_secondary_container,
        ),
        AppNotificationPriority::Normal => (
            material.roles.primary_container,
            material.roles.on_primary_container,
        ),
        AppNotificationPriority::High => (
            material.extended.warning.color_container,
            material.extended.warning.on_color_container,
        ),
        AppNotificationPriority::Critical => (
            material.roles.error_container,
            material.roles.on_error_container,
        ),
    }
}

fn notification_icon(icon: IconName, accent: u32) -> impl IntoElement {
    div()
        .flex_none()
        .size(px(NOTIFICATION_ICON_CONTAINER_SIZE))
        .rounded(px(999.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(color_with_alpha(accent, 0x18))
        .child(
            Icon::new(icon)
                .size(px(NOTIFICATION_ICON_SIZE))
                .text_color(rgb(accent)),
        )
}

fn notification_text(
    title: SharedString,
    message: SharedString,
    roles: miaominal_settings::theme::Md3Roles,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w(px(0.0))
        .gap_1()
        .child(
            div()
                .text_sm()
                .text_color(rgb(roles.on_surface))
                .child(title),
        )
        .child(
            div()
                .text_sm()
                .line_height(miaominal_settings::scaled_line_height(18.0))
                .text_color(rgb(roles.on_surface_variant))
                .child(message),
        )
}

fn style_notification(notification: Notification) -> Notification {
    let roles = miaominal_settings::current_theme().material.roles;
    notification
        .border_0()
        .bg(rgb(roles.surface_container_highest))
        .rounded(px(NOTIFICATION_CORNER_RADIUS))
}

pub(crate) fn render_notification_center_popover(cx: &App) -> impl IntoElement {
    let unread = notification_center_unread(cx);
    let priority = notification_center_highest_unread(cx);
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let badge_color = match priority {
        Some(AppNotificationPriority::Critical) => roles.error,
        Some(AppNotificationPriority::High) => material.extended.warning.color,
        Some(AppNotificationPriority::Normal) => roles.primary,
        Some(AppNotificationPriority::Low) | None => roles.secondary,
    };
    let trigger = NotificationCenterTrigger {
        selected: false,
        unread,
        badge_color,
        roles,
        tooltip: i18n::string("notifications.center.title").into(),
    };

    Popover::new("notification-center-popover")
        .anchor(Anchor::BottomRight)
        .appearance(false)
        .trigger(trigger)
        .on_open_change(|open, window, cx| {
            if *open {
                mark_all_notifications_read(cx);
                window.refresh();
            }
        })
        .content(|_, _, cx| {
            let entries = notification_center_entries(cx);
            let roles = miaominal_settings::current_theme().material.roles;
            let popover = cx.entity().clone();
            v_flex()
                .w(px(408.0))
                .max_h(px(480.0))
                .rounded(px(24.0))
                .bg(rgb(roles.surface_container_highest))
                .shadow_lg()
                .overflow_hidden()
                .child(
                    h_flex()
                        .w_full()
                        .min_h(px(52.0))
                        .px_4()
                        .pt_1()
                        .justify_between()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(roles.on_surface))
                                .child(i18n::string("notifications.center.title")),
                        )
                        .child(
                            Button::new("notification-center-clear")
                                .ghost()
                                .compact()
                                .rounded(px(20.0))
                                .label(i18n::string("notifications.center.clear_all"))
                                .on_click(|_, window, cx| {
                                    clear_all_notifications(cx);
                                    window.refresh();
                                }),
                        ),
                )
                .child(
                    v_flex()
                        .max_h(px(408.0))
                        .overflow_y_scrollbar()
                        .px_2()
                        .pb_2()
                        .gap_1()
                        .when(entries.is_empty(), |this| {
                            this.child(
                                v_flex()
                                    .h(px(152.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_3()
                                    .text_color(rgb(roles.on_surface_variant))
                                    .child(
                                        div()
                                            .size(px(48.0))
                                            .rounded(px(999.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .bg(color_with_alpha(roles.secondary, 0x20))
                                            .child(
                                                Icon::new(IconName::Bell)
                                                    .size(px(24.0))
                                                    .text_color(rgb(roles.secondary)),
                                            ),
                                    )
                                    .child(i18n::string("notifications.center.empty")),
                            )
                        })
                        .children(entries.into_iter().map(move |entry| {
                            render_notification_history_entry(entry, popover.clone())
                        })),
                )
                .with_animation(
                    "notification-center-panel-enter",
                    notification_panel_enter_animation(),
                    |element, delta| {
                        element
                            .opacity(delta)
                            .top(px((1.0 - delta) * NOTIFICATION_PANEL_ENTER_OFFSET))
                    },
                )
        })
}

fn render_notification_history_entry(
    entry: AppNotificationEntry,
    popover: Entity<PopoverState>,
) -> AnyElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let accent = notification_accent(entry.tone, &material);
    let (priority_background, priority_foreground) =
        notification_priority_colors(entry.priority, &material);
    let action = entry.action;
    let action_label = entry.action_label;
    let entry_id = entry.id;
    let is_dismissing = entry.dismissal_token.is_some();
    let remove_entry_id = entry_id.clone();
    let time = format_local_timestamp(Some(entry.created_at));
    let priority = notification_priority_label(entry.priority);
    let element = h_flex()
        .id(SharedString::from(format!(
            "notification-history-{}",
            entry_id
        )))
        .w_full()
        .items_start()
        .gap_3()
        .px_4()
        .pt_3()
        .pb_4()
        .rounded(px(18.0))
        .bg(rgb(roles.surface_container_high))
        .child(notification_icon(entry.tone.icon(), accent))
        .child(
            v_flex()
                .child(notification_text(
                    entry.title.into(),
                    entry.message.into(),
                    roles,
                ))
                .flex_1()
                .min_w(px(0.0))
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .child(badge(priority, priority_background, priority_foreground))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(roles.on_surface_variant))
                                .child(time),
                        ),
                )
                .when_some(action.zip(action_label), |this, (action, label)| {
                    let popover = popover.clone();
                    this.child(
                        h_flex().justify_end().child(
                            Button::new(SharedString::from(format!(
                                "notification-history-action-{entry_id}"
                            )))
                            .ghost()
                            .compact()
                            .rounded(px(20.0))
                            .label(label)
                            .on_click(move |_, window, cx| {
                                execute_notification_action(action, window, cx);
                                popover.update(cx, |state, cx| state.dismiss(window, cx));
                            }),
                        ),
                    )
                }),
        )
        .child(div().flex_none().child(icon_button(
            AppIcon::Close,
            NOTIFICATION_CLOSE_BUTTON_SIZE,
            NOTIFICATION_CLOSE_BUTTON_RADIUS,
            Some(roles.surface_container_high),
            Some(roles.on_surface_variant),
            None,
            move |_, cx| remove_notification(&remove_entry_id, cx),
        )));
    if is_dismissing {
        element
            .with_animation(
                SharedString::from(format!("notification-history-exit-{entry_id}")),
                notification_item_exit_animation(),
                |element, delta| {
                    element
                        .opacity(1.0 - delta)
                        .left(px(delta * NOTIFICATION_ITEM_EXIT_OFFSET))
                },
            )
            .into_any_element()
    } else {
        element
            .with_animation(
                SharedString::from(format!("notification-history-enter-{entry_id}")),
                notification_item_enter_animation(),
                |element, delta| {
                    element
                        .opacity(delta)
                        .top(px((1.0 - delta) * NOTIFICATION_ITEM_ENTER_OFFSET))
                },
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: Option<&str>, priority: AppNotificationPriority) -> AppNotification {
        let note = AppNotification::new(AppNotificationTone::Info, priority, "title", "message");
        id.map_or(note.clone(), |id| note.stable_id(id))
    }

    #[test]
    fn stable_ids_update_and_return_to_unread() {
        let mut center = NotificationCenterState::default();
        center.publish(&note(Some("other"), AppNotificationPriority::Normal));
        center.publish(&note(Some("same"), AppNotificationPriority::Low));
        center.mark_all_read();
        let mut updated = note(Some("same"), AppNotificationPriority::High);
        updated.message = "updated".into();
        center.publish(&updated);
        assert_eq!(center.entries.len(), 2);
        let entries = center.sorted_entries();
        assert_eq!(entries[0].id, "same");
        assert_eq!(entries[0].message, "updated");
        assert!(!entries[0].read);
    }

    #[test]
    fn anonymous_notifications_append_and_sort_by_priority_then_sequence() {
        let mut center = NotificationCenterState::default();
        center.publish(&note(None, AppNotificationPriority::Low));
        center.publish(&note(None, AppNotificationPriority::High));
        center.publish(&note(None, AppNotificationPriority::High));
        let entries = center.sorted_entries();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].priority, AppNotificationPriority::High);
        assert!(entries[0].sequence > entries[1].sequence);
    }

    #[test]
    fn history_is_bounded_and_clearable() {
        let mut center = NotificationCenterState::default();
        for _ in 0..=NOTIFICATION_HISTORY_LIMIT {
            center.publish(&note(None, AppNotificationPriority::Normal));
        }
        assert_eq!(center.entries.len(), NOTIFICATION_HISTORY_LIMIT);
        let removals = center.begin_clear_all();
        assert_eq!(center.entries.len(), NOTIFICATION_HISTORY_LIMIT);
        assert!(
            center
                .entries
                .iter()
                .all(|entry| entry.dismissal_token.is_some())
        );
        assert!(center.finish_removals(&removals));
        assert!(center.entries.is_empty());
    }

    #[test]
    fn individual_history_entries_can_be_removed() {
        let mut center = NotificationCenterState::default();
        center.publish(&note(Some("keep"), AppNotificationPriority::Low));
        center.publish(&note(Some("remove"), AppNotificationPriority::High));

        let removal = center
            .begin_removal("remove")
            .expect("existing notification starts its exit animation");
        assert_eq!(center.entries.len(), 2);
        assert!(
            center
                .entries
                .iter()
                .find(|entry| entry.id == "remove")
                .is_some_and(|entry| entry.dismissal_token.is_some())
        );
        assert!(center.begin_removal("remove").is_none());
        assert!(center.begin_removal("missing").is_none());
        assert!(center.finish_removals(&[removal]));
        assert_eq!(center.entries.len(), 1);
        assert_eq!(center.entries[0].id, "keep");
    }

    #[test]
    fn stable_id_republish_cancels_a_pending_removal() {
        let mut center = NotificationCenterState::default();
        center.publish(&note(Some("same"), AppNotificationPriority::Low));
        let removal = center
            .begin_removal("same")
            .expect("existing notification starts its exit animation");

        center.publish(&note(Some("same"), AppNotificationPriority::High));

        assert!(center.entries[0].dismissal_token.is_none());
        assert!(!center.finish_removals(&[removal]));
        assert_eq!(center.entries.len(), 1);
        assert_eq!(center.entries[0].priority, AppNotificationPriority::High);
    }

    #[test]
    fn mark_all_read_clears_unread_state() {
        let mut center = NotificationCenterState::default();
        center.publish(&note(None, AppNotificationPriority::Low));
        center.publish(&note(None, AppNotificationPriority::Critical));
        assert_eq!(center.unread_count(), 2);
        assert_eq!(
            center.highest_unread_priority(),
            Some(AppNotificationPriority::Critical)
        );

        center.mark_all_read();

        assert_eq!(center.unread_count(), 0);
        assert_eq!(center.highest_unread_priority(), None);
    }

    #[test]
    fn transient_toast_actions_are_not_saved_in_history() {
        let mut center = NotificationCenterState::default();
        let notification = note(Some("transient"), AppNotificationPriority::Normal)
            .toast_action("Retry", |_, _| {});
        center.publish(&notification);
        assert!(center.entries[0].action.is_none());
        assert!(center.entries[0].action_label.is_none());
    }

    #[test]
    fn structured_actions_are_preserved_for_history() {
        let mut center = NotificationCenterState::default();
        let notification = note(Some("sync"), AppNotificationPriority::High).structured_action(
            AppNotificationAction::OpenSyncSettings,
            "Open sync settings",
        );
        center.publish(&notification);
        assert_eq!(
            center.entries[0].action,
            Some(AppNotificationAction::OpenSyncSettings)
        );
        assert_eq!(
            center.entries[0].action_label.as_deref(),
            Some("Open sync settings")
        );
    }

    #[test]
    fn pending_toasts_survive_without_a_window_and_stable_ids_coalesce() {
        let mut center = NotificationCenterState::default();
        let mut first = note(Some("sync"), AppNotificationPriority::High);
        first.message = "first".into();
        center.enqueue_pending_toast(first);
        let mut updated = note(Some("sync"), AppNotificationPriority::High);
        updated.message = "updated".into();
        center.enqueue_pending_toast(updated);
        center.enqueue_pending_toast(note(None, AppNotificationPriority::Low));

        let pending = center.take_pending_toasts();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].message, "updated");
        assert!(center.pending_toasts.is_empty());
    }
}
