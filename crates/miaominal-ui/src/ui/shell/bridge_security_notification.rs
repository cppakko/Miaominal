use super::*;
use crate::ui::i18n;
use miaominal_core::ssh_bridge_security::{
    BridgePendingAuthorization, BridgePendingPhase, BridgeSecuritySnapshot,
};

const NOTIFICATION_WIDTH: f32 = 404.0;
const NOTIFICATION_HEIGHT: f32 = 184.0;
const NOTIFICATION_EDGE_MARGIN: f32 = 16.0;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::ui::shell) struct BridgeSecurityNotificationKey {
    pub request_id: String,
    pub phase: BridgePendingPhase,
}

impl From<&BridgePendingAuthorization> for BridgeSecurityNotificationKey {
    fn from(request: &BridgePendingAuthorization) -> Self {
        Self {
            request_id: request.request_id.clone(),
            phase: request.phase,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct BridgeSecurityNotificationModel {
    pub key: BridgeSecurityNotificationKey,
    pub profile_name: String,
    pub source_summary: String,
    pub source_icon_path: Option<String>,
    pub phase: BridgePendingPhase,
    pub expires_at: i64,
    pub additional_count: usize,
    pub vault_locked: bool,
}

impl BridgeSecurityNotificationModel {
    pub fn action(&self) -> BridgeSecurityNotificationAction {
        match self.phase {
            BridgePendingPhase::AwaitingVaultUnlock => {
                BridgeSecurityNotificationAction::UnlockVault
            }
            BridgePendingPhase::AwaitingApproval if self.vault_locked => {
                BridgeSecurityNotificationAction::ApproveAndUnlock
            }
            BridgePendingPhase::AwaitingApproval => BridgeSecurityNotificationAction::Approve,
            BridgePendingPhase::AwaitingSystemAuth => {
                BridgeSecurityNotificationAction::OpenSecurity
            }
        }
    }

    pub fn secondary_action(&self) -> Option<BridgeSecurityNotificationAction> {
        (self.phase == BridgePendingPhase::AwaitingApproval)
            .then_some(BridgeSecurityNotificationAction::Reject)
    }

    fn title(&self) -> String {
        i18n::string(match self.phase {
            BridgePendingPhase::AwaitingApproval => {
                "settings.openssh_integration.security.notification_approval_title"
            }
            BridgePendingPhase::AwaitingSystemAuth => {
                "settings.openssh_integration.security.notification_system_auth_title"
            }
            BridgePendingPhase::AwaitingVaultUnlock => {
                "settings.openssh_integration.security.notification_vault_title"
            }
        })
    }

    fn action_label(action: BridgeSecurityNotificationAction) -> String {
        i18n::string(match action {
            BridgeSecurityNotificationAction::OpenSecurity => {
                "settings.openssh_integration.security.view"
            }
            BridgeSecurityNotificationAction::UnlockVault => {
                "settings.openssh_integration.security.unlock_vault"
            }
            BridgeSecurityNotificationAction::Approve => {
                "settings.openssh_integration.security.approve"
            }
            BridgeSecurityNotificationAction::ApproveAndUnlock => {
                "settings.openssh_integration.security.approve_and_unlock"
            }
            BridgeSecurityNotificationAction::Reject => {
                "settings.openssh_integration.security.reject"
            }
        })
    }

    fn remaining_seconds(&self) -> i64 {
        self.expires_at
            .saturating_sub(time::OffsetDateTime::now_utc().unix_timestamp())
            .max(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum BridgeSecurityNotificationAction {
    OpenSecurity,
    UnlockVault,
    Approve,
    ApproveAndUnlock,
    Reject,
}

#[derive(Default)]
pub(in crate::ui::shell) struct BridgeSecurityNotificationState {
    headline: Option<BridgeSecurityNotificationKey>,
    dismissed: HashSet<BridgeSecurityNotificationKey>,
    presented: HashSet<BridgeSecurityNotificationKey>,
}

impl BridgeSecurityNotificationState {
    pub fn reconcile(
        &mut self,
        snapshot: &BridgeSecuritySnapshot,
        now: i64,
    ) -> Option<BridgeSecurityNotificationModel> {
        let mut pending = snapshot
            .pending
            .iter()
            .filter(|request| request.expires_at > now)
            .collect::<Vec<_>>();
        pending.sort_by(|left, right| {
            left.expires_at
                .cmp(&right.expires_at)
                .then_with(|| left.request_id.cmp(&right.request_id))
        });

        let active = pending
            .iter()
            .map(|request| BridgeSecurityNotificationKey::from(*request))
            .collect::<HashSet<_>>();
        self.dismissed.retain(|key| active.contains(key));
        self.presented.retain(|key| active.contains(key));

        let request = pending.first().copied()?;
        let key = BridgeSecurityNotificationKey::from(request);
        self.headline = Some(key.clone());
        let source_icon_path = request
            .peer
            .application_source_path()
            .map(notification_display_text);
        Some(BridgeSecurityNotificationModel {
            key,
            profile_name: notification_display_text(&request.profile_name),
            source_summary: source_icon_path.clone().unwrap_or_else(|| {
                i18n::string("settings.openssh_integration.security.unknown_source")
            }),
            source_icon_path,
            phase: request.phase,
            expires_at: request.expires_at,
            additional_count: pending.len().saturating_sub(1),
            vault_locked: false,
        })
    }

    pub fn clear(&mut self) {
        self.headline = None;
        self.dismissed.clear();
        self.presented.clear();
    }

    pub fn should_present(&self, key: &BridgeSecurityNotificationKey) -> bool {
        !self.dismissed.contains(key) && !self.presented.contains(key)
    }

    pub fn mark_presented(&mut self, key: BridgeSecurityNotificationKey) {
        self.presented.insert(key);
    }

    pub fn dismiss(&mut self, key: BridgeSecurityNotificationKey) {
        self.dismissed.insert(key.clone());
        self.presented.insert(key);
    }
}

fn notification_display_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn notification_wrappable_path(value: &str) -> String {
    let mut display = String::with_capacity(value.len());
    for character in value.chars() {
        display.push(character);
        if matches!(character, '/' | '\\') {
            display.push('\u{200b}');
        }
    }
    display
}

pub(in crate::ui::shell) struct BridgeSecurityNotificationView {
    model: BridgeSecurityNotificationModel,
    controller: WeakEntity<SettingsController>,
}

impl BridgeSecurityNotificationView {
    pub fn new(
        model: BridgeSecurityNotificationModel,
        controller: WeakEntity<SettingsController>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self { model, controller }
    }

    pub fn set_model(&mut self, model: BridgeSecurityNotificationModel, cx: &mut Context<Self>) {
        if self.model != model {
            self.model = model;
            cx.notify();
        }
    }
}

impl Render for BridgeSecurityNotificationView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let accent = material.extended.warning.color;
        let title = self.model.title();
        let action = self.model.action();
        let action_label = BridgeSecurityNotificationModel::action_label(action);
        let secondary_action = self.model.secondary_action();
        let secondary_action_label =
            secondary_action.map(BridgeSecurityNotificationModel::action_label);
        let source = i18n::string_args(
            "settings.openssh_integration.security.notification_source",
            &[(
                "source",
                &notification_wrappable_path(&self.model.source_summary),
            )],
        );
        let source_tooltip = self.model.source_summary.clone();
        let process_icon = self
            .model
            .source_icon_path
            .as_deref()
            .and_then(|path| render_system_path_icon(path, px(28.0), window, cx));
        let seconds = self.model.remaining_seconds().to_string();
        let expires = i18n::string_args(
            "settings.openssh_integration.security.notification_expires",
            &[("seconds", &seconds)],
        );
        let additional = (self.model.additional_count > 0).then(|| {
            let count = self.model.additional_count.to_string();
            i18n::string_args(
                "settings.openssh_integration.security.notification_additional",
                &[("count", &count)],
            )
        });
        let dismiss_key = self.model.key.clone();
        let dismiss_controller = self.controller.clone();
        let action_key = self.model.key.clone();
        let action_controller = self.controller.clone();
        let secondary_key = self.model.key.clone();
        let secondary_controller = self.controller.clone();

        div().size_full().child(
            v_flex()
                .size_full()
                .p_4()
                .gap_3()
                .rounded(px(20.0))
                .bg(rgb(roles.surface_container_highest))
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .gap_3()
                        .child(
                            div()
                                .flex_none()
                                .size(px(40.0))
                                .rounded(px(999.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(if process_icon.is_some() {
                                    rgb(roles.surface_container_low)
                                } else {
                                    color_with_alpha(accent, 0x20)
                                })
                                .child(process_icon.unwrap_or_else(|| {
                                    Icon::new(IconName::TriangleAlert)
                                        .size(px(24.0))
                                        .text_color(rgb(accent))
                                        .into_any_element()
                                })),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .line_height(miaominal_settings::scaled_line_height(20.0))
                                        .text_color(rgb(roles.on_surface))
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .line_clamp(1)
                                        .text_ellipsis()
                                        .text_color(rgb(roles.on_surface_variant))
                                        .child(self.model.profile_name.clone()),
                                )
                                .child(
                                    div()
                                        .id("bridge-security-window-source")
                                        .text_sm()
                                        .line_clamp(2)
                                        .text_ellipsis()
                                        .text_color(rgb(roles.on_surface_variant))
                                        .tooltip(move |window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(
                                                source_tooltip.clone(),
                                            )
                                            .build(window, cx)
                                        })
                                        .child(source),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .text_xs()
                                        .text_color(rgb(roles.on_surface_variant))
                                        .child(expires)
                                        .when_some(additional, |row, additional| {
                                            row.child("·").child(additional)
                                        }),
                                ),
                        )
                        .child(div().flex_none().child(icon_button(
                            AppIcon::Close,
                            30.0,
                            10.0,
                            Some(roles.surface_container_low),
                            Some(roles.on_surface_variant),
                            None,
                            move |window, cx| {
                                window.remove_window();
                                let _ = dismiss_controller.update(cx, |controller, cx| {
                                    controller.dismiss_bridge_security_notification(
                                        dismiss_key.clone(),
                                        cx,
                                    );
                                });
                            },
                        ))),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .when_some(
                            secondary_action.zip(secondary_action_label),
                            move |actions, (secondary_action, secondary_label)| {
                                actions.child(
                                    basic_dialog_action_button(
                                        "bridge-security-window-secondary-action",
                                        secondary_label,
                                        BasicDialogActionTone::Destructive,
                                    )
                                    .on_click(
                                        move |_, window, cx| {
                                            window.remove_window();
                                            let _ = secondary_controller.update(
                                                cx,
                                                |controller, cx| {
                                                    controller
                                                        .handle_bridge_security_notification_action(
                                                            secondary_key.clone(),
                                                            secondary_action,
                                                            cx,
                                                        );
                                                },
                                            );
                                        },
                                    ),
                                )
                            },
                        )
                        .child(
                            basic_dialog_action_button(
                                "bridge-security-window-action",
                                action_label,
                                BasicDialogActionTone::Default,
                            )
                            .on_click(move |_, window, cx| {
                                window.remove_window();
                                let _ = action_controller.update(cx, |controller, cx| {
                                    controller.handle_bridge_security_notification_action(
                                        action_key.clone(),
                                        action,
                                        cx,
                                    );
                                });
                            }),
                        ),
                ),
        )
    }
}

pub(in crate::ui::shell) fn bridge_security_notification_window_options(
    display: Option<&dyn gpui_kit::PlatformDisplay>,
) -> gpui_kit::WindowOptions {
    let display_bounds = display.map(gpui_kit::PlatformDisplay::visible_bounds);
    let available_width = display_bounds
        .map(|bounds| (bounds.size.width - px(NOTIFICATION_EDGE_MARGIN * 2.0)).max(px(240.0)))
        .unwrap_or(px(NOTIFICATION_WIDTH));
    let width = px(NOTIFICATION_WIDTH).min(available_width);
    let size = gpui_kit::size(width, px(NOTIFICATION_HEIGHT));
    let origin = display_bounds.map_or(gpui_kit::point(px(0.0), px(0.0)), |bounds| {
        gpui_kit::point(
            bounds.origin.x + bounds.size.width - size.width - px(NOTIFICATION_EDGE_MARGIN),
            bounds.origin.y + bounds.size.height - size.height - px(NOTIFICATION_EDGE_MARGIN),
        )
    });

    gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds::new(origin, size))),
        titlebar: None,
        focus: false,
        show: true,
        kind: bridge_security_notification_window_kind(),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: display.map(gpui_kit::PlatformDisplay::id),
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        window_min_size: Some(size),
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        window_decorations: Some(gpui_kit::WindowDecorations::Client),
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        app_id: Some("miaominal-ssh-bridge-notification".to_string()),
        ..Default::default()
    }
}

fn bridge_security_notification_window_kind() -> gpui_kit::WindowKind {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty()) {
        use gpui_kit::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};

        return gpui_kit::WindowKind::LayerShell(LayerShellOptions {
            namespace: "miaominal-ssh-bridge-notification".to_string(),
            layer: Layer::Overlay,
            anchor: Anchor::RIGHT | Anchor::BOTTOM,
            margin: Some((
                px(0.0),
                px(NOTIFICATION_EDGE_MARGIN),
                px(NOTIFICATION_EDGE_MARGIN),
                px(0.0),
            )),
            keyboard_interactivity: KeyboardInteractivity::None,
            ..Default::default()
        });
    }

    gpui_kit::WindowKind::PopUp
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::ssh_bridge_security::{
        BridgePeerIdentity, BridgeSecurityLevel, BridgeSecurityPolicy,
    };

    fn request(id: &str, phase: BridgePendingPhase, expires_at: i64) -> BridgePendingAuthorization {
        BridgePendingAuthorization {
            request_id: id.into(),
            profile_id: format!("profile-{id}"),
            profile_name: format!("Profile {id}"),
            level: BridgeSecurityLevel::RequireApproval { timeout_secs: 30 },
            phase,
            policy_generation: 1,
            peer: BridgePeerIdentity::default(),
            created_at: 1,
            phase_started_at: 1,
            expires_at,
        }
    }

    fn snapshot(pending: Vec<BridgePendingAuthorization>) -> BridgeSecuritySnapshot {
        BridgeSecuritySnapshot {
            policy: BridgeSecurityPolicy::default(),
            pending,
            audit_health_error: None,
            policy_store_error: None,
            system_auth_available: true,
        }
    }

    #[test]
    fn earliest_expiration_is_the_headline_with_stable_id_tiebreaker() {
        let mut state = BridgeSecurityNotificationState::default();
        let model = state
            .reconcile(
                &snapshot(vec![
                    request("z", BridgePendingPhase::AwaitingApproval, 30),
                    request("b", BridgePendingPhase::AwaitingApproval, 20),
                    request("a", BridgePendingPhase::AwaitingApproval, 20),
                ]),
                10,
            )
            .unwrap();
        assert_eq!(model.key.request_id, "a");
        assert_eq!(model.additional_count, 2);
    }

    #[test]
    fn dismissed_headline_does_not_immediately_rotate_to_an_existing_request() {
        let mut state = BridgeSecurityNotificationState::default();
        let first = snapshot(vec![
            request("first", BridgePendingPhase::AwaitingApproval, 20),
            request("second", BridgePendingPhase::AwaitingApproval, 30),
        ]);
        let headline = state.reconcile(&first, 10).unwrap();
        state.dismiss(headline.key.clone());
        let unchanged = state.reconcile(&first, 10).unwrap();
        assert_eq!(unchanged.key, headline.key);
        assert!(!state.should_present(&unchanged.key));

        let next = state
            .reconcile(
                &snapshot(vec![request(
                    "second",
                    BridgePendingPhase::AwaitingApproval,
                    30,
                )]),
                21,
            )
            .unwrap();
        assert_eq!(next.key.request_id, "second");
        assert!(state.should_present(&next.key));
    }

    #[test]
    fn phase_transition_is_a_new_notification_key() {
        let mut state = BridgeSecurityNotificationState::default();
        let approval = state
            .reconcile(
                &snapshot(vec![request(
                    "same",
                    BridgePendingPhase::AwaitingApproval,
                    30,
                )]),
                10,
            )
            .unwrap();
        state.mark_presented(approval.key.clone());
        let vault = state
            .reconcile(
                &snapshot(vec![request(
                    "same",
                    BridgePendingPhase::AwaitingVaultUnlock,
                    70,
                )]),
                20,
            )
            .unwrap();
        assert_ne!(approval.key, vault.key);
        assert!(state.should_present(&vault.key));
        assert_eq!(
            vault.action(),
            BridgeSecurityNotificationAction::UnlockVault
        );
    }

    #[test]
    fn approval_actions_reflect_the_local_vault_state() {
        let mut state = BridgeSecurityNotificationState::default();
        let mut model = state
            .reconcile(
                &snapshot(vec![request(
                    "approval",
                    BridgePendingPhase::AwaitingApproval,
                    30,
                )]),
                10,
            )
            .unwrap();
        assert_eq!(model.action(), BridgeSecurityNotificationAction::Approve);
        assert_eq!(
            model.secondary_action(),
            Some(BridgeSecurityNotificationAction::Reject)
        );

        model.vault_locked = true;
        assert_eq!(
            model.action(),
            BridgeSecurityNotificationAction::ApproveAndUnlock
        );
    }

    #[test]
    fn process_paths_receive_safe_wrap_points() {
        assert_eq!(
            notification_wrappable_path(r"C:\Windows\explorer.exe"),
            "C:\\\u{200b}Windows\\\u{200b}explorer.exe"
        );
    }

    #[test]
    fn presented_headline_is_not_repeated_until_it_leaves_pending() {
        let mut state = BridgeSecurityNotificationState::default();
        let pending = snapshot(vec![request(
            "same",
            BridgePendingPhase::AwaitingSystemAuth,
            30,
        )]);
        let model = state.reconcile(&pending, 10).unwrap();
        assert!(state.should_present(&model.key));
        state.mark_presented(model.key.clone());
        let repeated = state.reconcile(&pending, 11).unwrap();
        assert!(!state.should_present(&repeated.key));

        assert!(state.reconcile(&snapshot(vec![]), 11).is_none());
        let again = state.reconcile(&pending, 12).unwrap();
        assert!(state.should_present(&again.key));
    }

    #[test]
    fn expired_and_empty_snapshots_remove_notification_state() {
        let mut state = BridgeSecurityNotificationState::default();
        let model = state
            .reconcile(
                &snapshot(vec![request(
                    "expired",
                    BridgePendingPhase::AwaitingApproval,
                    10,
                )]),
                9,
            )
            .unwrap();
        state.mark_presented(model.key);
        assert!(
            state
                .reconcile(
                    &snapshot(vec![request(
                        "expired",
                        BridgePendingPhase::AwaitingApproval,
                        10,
                    )]),
                    10,
                )
                .is_none()
        );
        assert!(state.reconcile(&snapshot(vec![]), 10).is_none());
    }
}
