use super::*;
use crate::ui::components::empty_state::{EmptyStateTone, m3_tonal_emblem};
use crate::ui::components::shell_tokens::{ShellColors, space};
use crate::ui::components::tokens::color_with_alpha;
use crate::ui::components::{
    FontFamilyPickerState, SegmentedSwitch, font_family_picker, md3_switch,
};
use crate::ui::i18n;
use gpui_kit::base::motion::{
    MotionStatus, Presence, PresencePhase, PresenceSample, Transition, spring, transition,
};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Size;
use gpui_kit::component::button::ButtonCustomVariant;
use gpui_kit::component::collapsible::Collapsible;
use gpui_kit::component::stepper::{Stepper, StepperItem};
use gpui_kit::{Div, ElementId, Rgba, TestSupportExt as _, rems};
use miaominal_settings::{AppLanguage, TerminalRightClickBehavior, ThemeId, scaled_font_size};
use std::rc::Rc;
use std::time::Duration;

const ONBOARDING_TITLE_BAR_HEIGHT: f32 = 56.0;
const ONBOARDING_STEP_TRANSITION_OFFSET: f32 = 18.0;
const ONBOARDING_STEP_EXIT_DURATION: Duration = Duration::from_millis(120);
const ONBOARDING_STEP_ENTER_DURATION: Duration = Duration::from_millis(200);
const ONBOARDING_TRAFFIC_LIGHT_PADDING: f32 = 71.0;
// Match gpui-component's fixed 24px Size::Medium stepper indicator circle.
const ONBOARDING_STEPPER_INDICATOR_ICON_SIZE: f32 = 14.0;

#[derive(Clone, Copy)]
struct OnboardingStepRenderState {
    step: OnboardingStep,
    visibility: f32,
    offset_x: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnboardingStepStatus {
    Completed,
    Current,
    PendingReachable,
    PendingBlocked,
}

fn onboarding_window_controls_on_left() -> bool {
    cfg!(target_os = "macos")
}

fn show_onboarding_macos_traffic_light_space(window: &Window) -> bool {
    cfg!(target_os = "macos") && !window.is_fullscreen()
}

pub(in crate::ui) fn render_onboarding_page(
    settings_entity: Entity<SettingsController>,
    notification_layer: Option<AnyElement>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let (
        active_step,
        onboarding_steps,
        local_vault_status,
        can_go_back,
        can_advance,
        can_finish,
        font_family_picker,
        terminal_font_family_picker,
        font_fallbacks_input,
        seed_color_picker,
        terminal_right_click_behavior_select,
        current_interface_font_size,
        current_terminal_font_size,
        current_line_height,
        current_seed_color,
        current_shift_right_click_context_menu,
        language_select,
        current_language_label,
        current_theme,
        current_material,
        forms,
    ) = {
        let settings_controller = settings_entity.read(cx);
        let settings = settings_controller.settings();
        (
            settings_controller.onboarding_state().onboarding_step,
            settings_controller.onboarding_steps(),
            settings_controller.local_vault_status(),
            settings_controller.can_go_back_onboarding(),
            settings_controller.can_advance_onboarding(),
            settings_controller.can_finish_onboarding(),
            settings_controller.forms.font_family_picker.clone(),
            settings_controller
                .forms
                .terminal_font_family_picker
                .clone(),
            settings_controller.forms.font_fallbacks_input.clone(),
            settings_controller.forms.seed_color_picker.clone(),
            settings_controller
                .forms
                .terminal_right_click_behavior_select
                .clone(),
            format!("{:.1}", settings.interface_font_size),
            format!("{:.1}", settings.terminal_font_size),
            format!("{:.1}", settings.line_height),
            settings.seed_color.clone(),
            settings.terminal_shift_right_click_context_menu,
            settings_controller.forms.language_select.clone(),
            SharedString::from(settings.language.native_name()),
            settings.theme_id,
            miaominal_settings::Theme::from_settings(settings).material,
            settings_controller.forms(),
        )
    };
    let portable = onboarding_steps.contains(&OnboardingStep::Security);
    let step_render_state = onboarding_step_render_state(&settings_entity, portable, window, cx);
    let visible_step = step_render_state.step;
    let portable_vault_ready = local_vault_status == LocalVaultStatus::Unlocked;
    let desired_seed_color = rgb(current_material.source);

    if seed_color_picker.read(cx).value() != Some(desired_seed_color.into()) {
        seed_color_picker.update(cx, |picker, cx| {
            picker.set_value(desired_seed_color, window, cx);
        });
    }

    let visible_step_index = visible_step.index(portable).unwrap_or_default() + 1;
    let total_steps = onboarding_steps.len();

    let step_content = match visible_step {
        OnboardingStep::Welcome => render_onboarding_welcome_step(language_select),
        OnboardingStep::Preferences => render_onboarding_preferences_step(
            current_theme,
            current_seed_color.clone(),
            current_material,
            font_family_picker,
            terminal_font_family_picker,
            font_fallbacks_input,
            seed_color_picker,
            terminal_right_click_behavior_select,
            current_interface_font_size,
            current_terminal_font_size,
            current_line_height,
            current_shift_right_click_context_menu,
            settings_entity.clone(),
            window,
            cx,
        ),
        OnboardingStep::Security => render_onboarding_security_step(
            visible_step_index,
            total_steps,
            local_vault_status,
            settings_entity.clone(),
            window,
            cx,
        ),
        OnboardingStep::Import => render_onboarding_import_step(
            visible_step_index,
            total_steps,
            forms,
            settings_entity.clone(),
            cx,
        ),
        OnboardingStep::Finish => render_onboarding_finish_step(
            current_language_label,
            current_theme,
            current_seed_color,
            current_material.source,
            portable,
            portable_vault_ready,
        ),
    };

    let stage = render_onboarding_stage(
        active_step,
        visible_step,
        step_render_state,
        onboarding_steps,
        portable,
        portable_vault_ready,
        can_go_back,
        can_advance,
        can_finish,
        step_content,
        settings_entity.clone(),
        window,
        cx,
    );

    let body = v_flex()
        .flex_1()
        .w_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .child(render_onboarding_stepper_bar(
            active_step,
            settings_entity,
            onboarding_steps,
            portable_vault_ready,
        ))
        .child(
            div()
                .flex_1()
                .w_full()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .px_3()
                .pb_3()
                .child(stage),
        );

    div()
        .size_full()
        .relative()
        .bg(rgb(roles.surface_container))
        .child(
            v_flex()
                .size_full()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .overflow_hidden()
                .child(render_onboarding_title_bar(portable, window))
                .child(body),
        )
        .when_some(notification_layer, |this, layer| this.child(layer))
        .into_any_element()
}

fn onboarding_step_render_state(
    settings: &Entity<SettingsController>,
    portable: bool,
    window: &mut Window,
    cx: &mut App,
) -> OnboardingStepRenderState {
    let previous = settings.read(cx).onboarding_state();
    let mut onboarding = previous;
    let direction_state =
        window.use_keyed_state("onboarding-step-motion-direction", cx, |_, _| 1.0_f32);
    if onboarding.visible_onboarding_step != onboarding.onboarding_step {
        let should_update_direction = match onboarding.onboarding_step_transition {
            None => true,
            Some(transition) => transition.phase == OnboardingStepTransitionPhase::Entering,
        };
        if should_update_direction {
            let from_index = onboarding
                .visible_onboarding_step
                .index(portable)
                .unwrap_or_default();
            let to_index = onboarding
                .onboarding_step
                .index(portable)
                .unwrap_or_default();
            let next_direction = if to_index >= from_index { 1.0 } else { -1.0 };
            if (*direction_state.read(cx) - next_direction).abs() > f32::EPSILON {
                direction_state.update(cx, |direction, _| *direction = next_direction);
            }
        }
    }
    let direction = *direction_state.read(cx);
    let render_state = resolve_onboarding_step_render_state(
        &mut onboarding,
        direction,
        window,
        cx.reduce_motion(),
    );
    if onboarding != previous {
        settings.update(cx, |controller, cx| {
            controller.replace_onboarding_state(onboarding);
            cx.notify();
        });
    }
    render_state
}

fn resolve_onboarding_step_render_state(
    onboarding: &mut OnboardingState,
    direction: f32,
    window: &mut Window,
    reduce_motion: bool,
) -> OnboardingStepRenderState {
    let now = Instant::now();
    let desired_step = onboarding.onboarding_step;

    if reduce_motion {
        onboarding.visible_onboarding_step = desired_step;
        onboarding.onboarding_step_transition = None;
        return OnboardingStepRenderState {
            step: onboarding.visible_onboarding_step,
            visibility: 1.0,
            offset_x: 0.0,
        };
    }

    if onboarding.visible_onboarding_step != desired_step {
        match onboarding.onboarding_step_transition {
            Some(transition) if transition.phase == OnboardingStepTransitionPhase::Exiting => {}
            _ => {
                onboarding.onboarding_step_transition = Some(OnboardingStepTransition {
                    phase: OnboardingStepTransitionPhase::Exiting,
                    started_at: now,
                    duration: ONBOARDING_STEP_EXIT_DURATION,
                });
            }
        }
    }

    if let Some(transition) = onboarding.onboarding_step_transition {
        let duration_seconds = transition.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            onboarding.visible_onboarding_step = desired_step;
            onboarding.onboarding_step_transition = None;

            return OnboardingStepRenderState {
                step: onboarding.visible_onboarding_step,
                visibility: 1.0,
                offset_x: 0.0,
            };
        }

        let elapsed = now.saturating_duration_since(transition.started_at);
        let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);

        if progress >= 1.0 {
            match transition.phase {
                OnboardingStepTransitionPhase::Exiting => {
                    onboarding.visible_onboarding_step = desired_step;
                    onboarding.onboarding_step_transition = Some(OnboardingStepTransition {
                        phase: OnboardingStepTransitionPhase::Entering,
                        started_at: now,
                        duration: ONBOARDING_STEP_ENTER_DURATION,
                    });
                    window.request_animation_frame();

                    return OnboardingStepRenderState {
                        step: onboarding.visible_onboarding_step,
                        visibility: 0.0,
                        offset_x: direction * ONBOARDING_STEP_TRANSITION_OFFSET,
                    };
                }
                OnboardingStepTransitionPhase::Entering => {
                    onboarding.visible_onboarding_step = desired_step;
                    onboarding.onboarding_step_transition = None;

                    return OnboardingStepRenderState {
                        step: onboarding.visible_onboarding_step,
                        visibility: 1.0,
                        offset_x: 0.0,
                    };
                }
            }
        }

        window.request_animation_frame();

        return match transition.phase {
            OnboardingStepTransitionPhase::Exiting => {
                let accelerate = progress * progress;
                let visibility = 1.0 - accelerate;
                OnboardingStepRenderState {
                    step: onboarding.visible_onboarding_step,
                    visibility,
                    offset_x: -direction * accelerate * ONBOARDING_STEP_TRANSITION_OFFSET,
                }
            }
            OnboardingStepTransitionPhase::Entering => {
                let decelerate = 1.0 - (1.0 - progress).powi(3);
                OnboardingStepRenderState {
                    step: onboarding.visible_onboarding_step,
                    visibility: decelerate,
                    offset_x: direction * (1.0 - decelerate) * ONBOARDING_STEP_TRANSITION_OFFSET,
                }
            }
        };
    }

    onboarding.visible_onboarding_step = desired_step;

    OnboardingStepRenderState {
        step: onboarding.visible_onboarding_step,
        visibility: 1.0,
        offset_x: 0.0,
    }
}

fn render_onboarding_title_bar(portable: bool, window: &Window) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    h_flex()
        .relative()
        .h(px(ONBOARDING_TITLE_BAR_HEIGHT))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .px_4()
        .bg(rgb(roles.surface_container))
        .when(
            cfg!(any(target_os = "linux", target_os = "freebsd")),
            |this| {
                this.on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, window, cx| {
                    if event.click_count == 1 {
                        window.start_window_move();
                    }
                    cx.stop_propagation();
                })
            },
        )
        .child(
            div()
                .absolute()
                .top(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .left(px(0.0))
                .window_control_area(WindowControlArea::Drag)
                .on_mouse_up(MouseButton::Left, |event: &MouseUpEvent, window, _| {
                    if event.click_count == 2 {
                        if cfg!(target_os = "macos") {
                            window.titlebar_double_click();
                        } else {
                            window.zoom_window();
                        }
                    }
                }),
        )
        .when(show_onboarding_macos_traffic_light_space(window), |this| {
            this.child(onboarding_window_controls(window))
        })
        .child(
            h_flex()
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .gap_3()
                .items_center()
                .child(Icon::new(AppIcon::Miaominal).size(px(30.0)))
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Title.scaled())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(roles.on_surface))
                        .child("Miaominal"),
                )
                .when(portable, |this| {
                    this.child(onboarding_status_chip(
                        None,
                        i18n::string("onboarding.rail.portable_badge"),
                        rgb(roles.secondary_container),
                        rgb(roles.on_secondary_container),
                    ))
                }),
        )
        .when(!onboarding_window_controls_on_left(), |this| {
            this.child(onboarding_window_controls(window))
        })
        .into_any_element()
}

fn onboarding_maximize_window_control_button(window: &Window) -> impl IntoElement {
    let is_zoomed = if cfg!(target_os = "macos") {
        window.is_fullscreen()
    } else {
        window.is_maximized()
    };

    let icon = if is_zoomed {
        AppIcon::Restore
    } else {
        AppIcon::Maximize
    };

    onboarding_window_control_button(
        "onboarding-window-maximize",
        icon,
        WindowControlArea::Max,
        |window, _| {
            if cfg!(target_os = "macos") {
                window.toggle_fullscreen();
            } else {
                window.zoom_window();
            }
        },
    )
}

fn onboarding_window_controls(window: &Window) -> impl IntoElement {
    if cfg!(target_os = "macos") {
        return div()
            .w(px(ONBOARDING_TRAFFIC_LIGHT_PADDING))
            .h_full()
            .flex_shrink_0()
            .into_any_element();
    }

    if onboarding_window_controls_on_left() {
        h_flex()
            .relative()
            .items_center()
            .gap(px(8.0))
            .child(onboarding_window_control_button(
                "onboarding-window-close",
                AppIcon::Close,
                WindowControlArea::Close,
                |window, cx| {
                    crate::ui::request_main_window_close(window, cx);
                },
            ))
            .child(onboarding_window_control_button(
                "onboarding-window-minimize",
                AppIcon::Minimize,
                WindowControlArea::Min,
                |window, _| {
                    window.minimize_window();
                },
            ))
            .child(onboarding_maximize_window_control_button(window))
            .into_any_element()
    } else {
        h_flex()
            .relative()
            .items_center()
            .gap(px(8.0))
            .child(onboarding_window_control_button(
                "onboarding-window-minimize",
                AppIcon::Minimize,
                WindowControlArea::Min,
                |window, _| {
                    window.minimize_window();
                },
            ))
            .child(onboarding_maximize_window_control_button(window))
            .child(onboarding_window_control_button(
                "onboarding-window-close",
                AppIcon::Close,
                WindowControlArea::Close,
                |window, cx| {
                    crate::ui::request_main_window_close(window, cx);
                },
            ))
            .into_any_element()
    }
}

fn onboarding_window_control_button(
    id: &'static str,
    icon: AppIcon,
    control_area: WindowControlArea,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let is_windows = cfg!(target_os = "windows");

    div()
        .id(SharedString::from(id))
        .size(px(36.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(roles.surface_container_highest))
        .text_color(rgb(roles.on_surface))
        .cursor_pointer()
        .active(|this| this.opacity(0.85))
        .occlude()
        .when(is_windows, |this| this.window_control_area(control_area))
        .when(!is_windows, |this| {
            this.on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_mouse_up(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
        })
        .child(Icon::from(icon).small())
        .on_click(move |_, window, cx| on_click(window, cx))
}

fn onboarding_step_status(
    step: OnboardingStep,
    active_step: OnboardingStep,
    portable: bool,
    portable_vault_ready: bool,
) -> OnboardingStepStatus {
    if step == active_step {
        return OnboardingStepStatus::Current;
    }

    let current_index = active_step.index(portable).unwrap_or_default();
    let step_index = step.index(portable).unwrap_or_default();
    if step_index < current_index {
        return OnboardingStepStatus::Completed;
    }

    let security_index = OnboardingStep::Security.index(true).unwrap_or(usize::MAX);
    let can_visit = !portable || portable_vault_ready || step_index <= security_index;
    if can_visit {
        OnboardingStepStatus::PendingReachable
    } else {
        OnboardingStepStatus::PendingBlocked
    }
}

fn step_element_id(step: OnboardingStep) -> &'static str {
    match step {
        OnboardingStep::Welcome => "onboarding-step-welcome",
        OnboardingStep::Preferences => "onboarding-step-preferences",
        OnboardingStep::Security => "onboarding-step-security",
        OnboardingStep::Import => "onboarding-step-import",
        OnboardingStep::Finish => "onboarding-step-finish",
    }
}

fn build_onboarding_stepper(
    active_step: OnboardingStep,
    steps: &'static [OnboardingStep],
    portable_vault_ready: bool,
    on_select: impl Fn(OnboardingStep, &mut App) + 'static,
) -> Stepper {
    let roles = miaominal_settings::current_theme().material.roles;
    let portable = steps.contains(&OnboardingStep::Security);
    let current_index = active_step.index(portable).unwrap_or_default();
    let on_select = Rc::new(on_select);

    Stepper::new("onboarding-stepper")
        .selected_index(current_index)
        .text_center(true)
        .with_size(Size::Medium)
        .items(steps.iter().copied().map(|step| {
            let status = onboarding_step_status(step, active_step, portable, portable_vault_ready);
            let (label_color, label_weight) = match status {
                OnboardingStepStatus::Current => (roles.on_surface, FontWeight::SEMIBOLD),
                OnboardingStepStatus::Completed => (roles.on_surface, FontWeight::MEDIUM),
                OnboardingStepStatus::PendingReachable | OnboardingStepStatus::PendingBlocked => {
                    (roles.on_surface_variant, FontWeight::NORMAL)
                }
            };

            let content = v_flex().items_center().child(
                div()
                    .id(SharedString::from(step_element_id(step)))
                    .test_support()
                    .text_center()
                    .text_color(rgb(label_color))
                    .font_weight(label_weight)
                    .child(i18n::string(step_label_key(step))),
            );

            let mut item = StepperItem::new().child(content);

            match status {
                OnboardingStepStatus::Completed => {
                    item = item.icon(
                        Icon::new(AppIcon::Check).size(px(ONBOARDING_STEPPER_INDICATOR_ICON_SIZE)),
                    );
                }
                OnboardingStepStatus::PendingBlocked => {
                    item = item
                        .icon(
                            Icon::new(AppIcon::Lock)
                                .size(px(ONBOARDING_STEPPER_INDICATOR_ICON_SIZE)),
                        )
                        .opacity(0.45);
                }
                OnboardingStepStatus::Current | OnboardingStepStatus::PendingReachable => {}
            }

            item
        }))
        .on_click(move |step_index, _, cx| {
            if let Some(&step) = steps.get(*step_index) {
                let status =
                    onboarding_step_status(step, active_step, portable, portable_vault_ready);
                if matches!(
                    status,
                    OnboardingStepStatus::Completed | OnboardingStepStatus::PendingReachable
                ) {
                    on_select(step, cx);
                }
            }
        })
}

fn render_onboarding_stepper_bar(
    active_step: OnboardingStep,
    settings: Entity<SettingsController>,
    steps: &'static [OnboardingStep],
    portable_vault_ready: bool,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    div()
        .id("onboarding-stepper-bar")
        .test_support()
        .w_full()
        .flex_shrink_0()
        .bg(rgb(roles.surface_container))
        .child(
            div()
                .w_full()
                .max_w(rems(48.0))
                .mx_auto()
                .px_4()
                .py_3()
                .child(build_onboarding_stepper(
                    active_step,
                    steps,
                    portable_vault_ready,
                    move |step, cx| {
                        settings.update(cx, |controller, cx| {
                            controller.set_onboarding_step(step, cx);
                        });
                    },
                )),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_onboarding_stage(
    active_step: OnboardingStep,
    visible_step: OnboardingStep,
    step_render_state: OnboardingStepRenderState,
    steps: &'static [OnboardingStep],
    portable: bool,
    portable_vault_ready: bool,
    can_go_back: bool,
    can_advance: bool,
    can_finish: bool,
    step_content: AnyElement,
    settings: Entity<SettingsController>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let is_focused_step = !matches!(visible_step, OnboardingStep::Preferences);
    let step_index = visible_step.index(portable).unwrap_or_default() + 1;
    let total_steps = steps.len();

    v_flex()
        .size_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .rounded(space(28.0))
        .bg(rgb(roles.surface))
        .overflow_hidden()
        .child(
            div()
                .flex_1()
                .w_full()
                .min_h(px(0.0))
                .overflow_hidden()
                .child(
                    div().size_full().overflow_y_scrollbar().child(
                        v_flex()
                            .w_full()
                            .min_h_full()
                            .max_w(rems(48.0))
                            .mx_auto()
                            .px_8()
                            .py_6()
                            .child(
                                v_flex()
                                    .relative()
                                    .flex_1()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .gap_6()
                                    .when(is_focused_step, |this| {
                                        this.items_center().justify_center()
                                    })
                                    .when(!is_focused_step, |this| this.items_start())
                                    .opacity(step_render_state.visibility)
                                    .left(px(step_render_state.offset_x))
                                    .when(!is_focused_step, |this| {
                                        this.child(render_onboarding_step_header(
                                            visible_step,
                                            step_index,
                                            total_steps,
                                            false,
                                        ))
                                    })
                                    .child(step_content),
                            ),
                    ),
                ),
        )
        .child(render_onboarding_footer(
            active_step,
            portable,
            portable_vault_ready,
            can_go_back,
            can_advance,
            can_finish,
            settings,
            window,
            cx,
        ))
        .into_any_element()
}

fn render_onboarding_step_header(
    step: OnboardingStep,
    step_index: usize,
    total_steps: usize,
    centered: bool,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let subtitle_key = step_subtitle_key(step);

    v_flex()
        .w_full()
        .gap(space(6.0))
        .when(centered, |this| this.items_center())
        .when(!centered, |this| this.items_start())
        .child(
            div()
                .when(centered, |this| this.text_center())
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(roles.primary))
                .child(i18n::string_args(
                    "onboarding.navigation.step_counter",
                    &[
                        ("current", &step_index.to_string()),
                        ("total", &total_steps.to_string()),
                    ],
                )),
        )
        .child(
            div()
                .when(centered, |this| this.text_center())
                .text_size(miaominal_settings::FontSize::DisplayLarge.scaled())
                .line_height(miaominal_settings::scaled_line_height(32.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(roles.on_surface))
                .child(i18n::string(step_title_key(step))),
        )
        .when_some(subtitle_key, |this, subtitle_key| {
            this.child(
                div()
                    .when(centered, |this| this.text_center())
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .line_height(miaominal_settings::scaled_line_height(21.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string(subtitle_key)),
            )
        })
        .into_any_element()
}

fn render_onboarding_welcome_step(
    language_select: Entity<SelectState<Vec<SelectOption<AppLanguage>>>>,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    v_flex()
        .w_full()
        .items_center()
        .gap_6()
        .child(m3_tonal_emblem(
            AppIcon::Miaominal,
            132.0,
            88.0,
            46.0,
            EmptyStateTone::Primary,
        ))
        .child(
            v_flex()
                .w_full()
                .max_w(rems(34.0))
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::DisplayLarge.scaled())
                        .line_height(miaominal_settings::scaled_line_height(32.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_center()
                        .text_color(rgb(roles.on_surface))
                        .child(i18n::string("onboarding.hero.title")),
                )
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Heading.scaled())
                        .line_height(miaominal_settings::scaled_line_height(21.0))
                        .text_center()
                        .text_color(rgb(roles.on_surface_variant))
                        .child(i18n::string("onboarding.hero.subtitle")),
                ),
        )
        .child(
            div().w_full().max_w(rems(30.0)).child(onboarding_panel(
                i18n::string("settings.appearance.language.label"),
                i18n::string("settings.appearance.language.description"),
                md3_select(&language_select)
                    .with_size(Size::Medium)
                    .w_full()
                    .into_any_element(),
            )),
        )
        .into_any_element()
}

fn render_onboarding_import_step(
    step_index: usize,
    total_steps: usize,
    settings_forms: SettingsForms,
    settings: Entity<SettingsController>,
    cx: &App,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let settings_import = settings.clone();

    v_flex()
        .w_full()
        .max_w(rems(40.0))
        .items_center()
        .gap_6()
        .child(render_onboarding_step_header(
            OnboardingStep::Import,
            step_index,
            total_steps,
            true,
        ))
        .child(onboarding_filled_surface(
            v_flex()
                .w_full()
                .gap_5()
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .justify_between()
                        .gap_3()
                        .child(div().flex_1().min_w(px(0.0)).child(onboarding_panel_header(
                            i18n::string("settings.connections.import_source.label"),
                            i18n::string("settings.connections.import_action.description"),
                        )))
                        .child(onboarding_status_chip(
                            None,
                            i18n::string("onboarding.rail.step_optional"),
                            rgb(roles.surface_container_highest),
                            rgb(roles.on_surface_variant),
                        )),
                )
                .child(
                    md3_select(&settings_forms.profile_import_source_select)
                        .with_size(Size::Medium)
                        .w_full(),
                )
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .flex_wrap()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(rems(16.0))
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(miaominal_settings::FontSize::Input.scaled())
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(roles.on_surface_variant))
                                        .child(i18n::string("onboarding.import.supported_sources")),
                                )
                                .child(
                                    h_flex().gap_2().flex_wrap().children(
                                        [
                                            i18n::string(
                                                "settings.connections.import_sources.openssh",
                                            ),
                                            i18n::string(
                                                "settings.connections.import_sources.putty",
                                            ),
                                            i18n::string(
                                                "settings.connections.import_sources.securecrt",
                                            ),
                                            i18n::string(
                                                "settings.connections.import_sources.finalshell",
                                            ),
                                        ]
                                        .into_iter()
                                        .map(onboarding_assist_chip),
                                    ),
                                ),
                        )
                        .child(onboarding_tonal_button(
                            "onboarding-import",
                            i18n::string("settings.connections.import_action.action"),
                            Some(AppIcon::Upload),
                            cx,
                            move |_window, cx| {
                                settings_import.update(cx, |controller, cx| {
                                    controller.request_profile_import(cx);
                                });
                            },
                        )),
                )
                .into_any_element(),
        ))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_onboarding_preferences_step(
    current_theme: ThemeId,
    current_seed_color: String,
    current_material: crate::ui::theme::MaterialTheme,
    font_family_picker: Entity<FontFamilyPickerState>,
    terminal_font_family_picker: Entity<FontFamilyPickerState>,
    font_fallbacks_input: Entity<InputState>,
    seed_color_picker: Entity<ColorPickerState>,
    terminal_right_click_behavior_select: Entity<
        SelectState<Vec<SelectOption<TerminalRightClickBehavior>>>,
    >,
    current_interface_font_size: String,
    current_terminal_font_size: String,
    current_line_height: String,
    current_shift_right_click_context_menu: bool,
    settings: Entity<SettingsController>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let roles = current_material.roles;
    let section_background = roles.surface_container;
    let section_foreground = roles.on_surface;
    let scale = (f32::from(window.rem_size()) / 14.0).max(f32::EPSILON);
    let advanced_state =
        window.use_keyed_state("onboarding-preferences-advanced", cx, |_, _| false);
    let advanced_open = *advanced_state.read(cx);
    let advanced_toggle = advanced_state.clone();

    v_flex()
        .w_full()
        .gap_4()
        .child(onboarding_category_card(
            i18n::string("settings.appearance.theme_group.title"),
            i18n::string("settings.appearance.theme_group.description"),
            section_background,
            section_foreground,
            h_flex()
                .w_full()
                .gap_5()
                .flex_wrap()
                .items_start()
                .child(onboarding_field(
                    i18n::string("settings.appearance.dark_mode.label"),
                    onboarding_theme_control(current_theme, scale, settings.clone()),
                ))
                .child(onboarding_field(
                    i18n::string("settings.appearance.seed_color.label"),
                    onboarding_seed_color_control(
                        current_seed_color,
                        current_material,
                        seed_color_picker,
                        settings.clone(),
                    ),
                ))
                .into_any_element(),
        ))
        .child(onboarding_category_card(
            i18n::string("settings.appearance.typography_group.title"),
            i18n::string("settings.appearance.typography_group.description"),
            section_background,
            section_foreground,
            h_flex()
                .w_full()
                .gap_5()
                .flex_wrap()
                .items_start()
                .child(
                    v_flex()
                        .min_w(rems(16.0))
                        .flex_1()
                        .gap_4()
                        .child(onboarding_field(
                            i18n::string("settings.appearance.font_family.label"),
                            onboarding_font_family_control(
                                font_family_picker,
                                settings.clone(),
                                cx,
                            ),
                        ))
                        .child(onboarding_field(
                            i18n::string("settings.appearance.interface_font_size.label"),
                            onboarding_interface_font_size_stepper(
                                current_interface_font_size,
                                settings.clone(),
                                cx,
                            ),
                        )),
                )
                .child(
                    v_flex()
                        .min_w(rems(16.0))
                        .flex_1()
                        .gap_4()
                        .child(onboarding_field(
                            i18n::string("settings.appearance.terminal_font_family.label"),
                            onboarding_terminal_font_family_control(
                                terminal_font_family_picker,
                                settings.clone(),
                                cx,
                            ),
                        ))
                        .child(onboarding_field(
                            i18n::string("settings.appearance.terminal_font_size.label"),
                            onboarding_terminal_font_size_stepper(
                                current_terminal_font_size,
                                settings.clone(),
                                cx,
                            ),
                        )),
                )
                .into_any_element(),
        ))
        .child(
            Collapsible::new()
                .motion_id("onboarding-preferences-advanced")
                .open(advanced_open)
                .w_full()
                .rounded(space(28.0))
                .bg(rgb(roles.surface_container))
                .overflow_hidden()
                .child(
                    Button::new("onboarding-preferences-advanced-toggle")
                        .ghost()
                        .w_full()
                        .h_auto()
                        .px_6()
                        .py_5()
                        .rounded(scaled_font_size(28.0))
                        .accessibility_label(i18n::string("onboarding.preferences.advanced_title"))
                        .on_click(move |_, _, cx| {
                            advanced_toggle.update(cx, |open, cx| {
                                *open = !*open;
                                cx.notify();
                            });
                        })
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .items_start()
                                        .gap(space(2.0))
                                        .child(
                                            div()
                                                .text_size(
                                                    miaominal_settings::FontSize::Subtitle.scaled(),
                                                )
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(rgb(roles.on_surface))
                                                .child(i18n::string(
                                                    "onboarding.preferences.advanced_title",
                                                )),
                                        )
                                        .child(
                                            div()
                                                .text_size(
                                                    miaominal_settings::FontSize::Input.scaled(),
                                                )
                                                .text_color(rgb(roles.on_surface_variant))
                                                .child(i18n::string(
                                                    "onboarding.preferences.advanced_description",
                                                )),
                                        ),
                                )
                                .child(
                                    Icon::new(if advanced_open {
                                        AppIcon::ChevronUp
                                    } else {
                                        AppIcon::ChevronDown
                                    })
                                    .small()
                                    .text_color(rgb(roles.on_surface_variant)),
                                ),
                        ),
                )
                .content(
                    v_flex()
                        .w_full()
                        .gap_5()
                        .px_6()
                        .pt_1()
                        .pb_6()
                        .child(
                            h_flex()
                                .w_full()
                                .gap_5()
                                .flex_wrap()
                                .items_start()
                                .child(onboarding_field_with_description(
                                    i18n::string("settings.appearance.font_fallbacks.label"),
                                    i18n::string("settings.appearance.font_fallbacks.description"),
                                    onboarding_font_fallbacks_control(
                                        font_fallbacks_input,
                                        settings.clone(),
                                    ),
                                ))
                                .child(onboarding_field(
                                    i18n::string("settings.appearance.line_height.label"),
                                    onboarding_line_height_stepper(
                                        current_line_height,
                                        settings.clone(),
                                        cx,
                                    ),
                                )),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .gap_5()
                                .flex_wrap()
                                .items_start()
                                .child(onboarding_field_with_description(
                                    i18n::string("settings.key_bindings.right_click.label"),
                                    i18n::string("settings.key_bindings.right_click.description"),
                                    onboarding_right_click_behavior_control(
                                        terminal_right_click_behavior_select,
                                    ),
                                ))
                                .child(onboarding_field_with_description(
                                    i18n::string("settings.key_bindings.shift_right_click.label"),
                                    i18n::string(
                                        "settings.key_bindings.shift_right_click.description",
                                    ),
                                    onboarding_shift_right_click_control(
                                        current_shift_right_click_context_menu,
                                        settings,
                                    ),
                                )),
                        ),
                ),
        )
        .into_any_element()
}

fn render_onboarding_finish_step(
    language_label: SharedString,
    theme_id: ThemeId,
    seed_color: String,
    seed_rgb: u32,
    portable: bool,
    portable_vault_ready: bool,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let theme_label = match theme_id {
        ThemeId::Light => i18n::string("enum.theme.light"),
        ThemeId::Dark => i18n::string("enum.theme.dark"),
    };

    v_flex()
        .w_full()
        .gap_6()
        .items_center()
        .justify_center()
        .child(m3_tonal_emblem(
            AppIcon::Sparkles,
            132.0,
            88.0,
            42.0,
            EmptyStateTone::Primary,
        ))
        .child(
            v_flex()
                .w_full()
                .max_w(rems(34.0))
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::DisplayLarge.scaled())
                        .line_height(miaominal_settings::scaled_line_height(32.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_center()
                        .text_color(rgb(roles.on_surface))
                        .child(i18n::string("onboarding.finish.title")),
                )
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Heading.scaled())
                        .line_height(miaominal_settings::scaled_line_height(21.0))
                        .text_color(rgb(roles.on_surface_variant))
                        .text_center()
                        .child(i18n::string(
                            "onboarding.finish.congratulations_description",
                        )),
                ),
        )
        .child(
            div().w_full().max_w(rems(34.0)).child(
                v_flex()
                    .w_full()
                    .gap_4()
                    .p_6()
                    .rounded(space(28.0))
                    .bg(rgb(roles.surface_container))
                    .child(onboarding_summary_row(
                        i18n::string("onboarding.finish.summary_language"),
                        language_label,
                        None,
                    ))
                    .child(onboarding_summary_row(
                        i18n::string("onboarding.finish.summary_theme"),
                        SharedString::from(format!("{theme_label} · {seed_color}")),
                        Some(seed_rgb),
                    ))
                    .when(portable && portable_vault_ready, |this| {
                        this.child(onboarding_summary_row(
                            i18n::string("onboarding.finish.summary_security"),
                            SharedString::from(i18n::string(
                                "onboarding.finish.summary_security_ready",
                            )),
                            None,
                        ))
                    }),
            ),
        )
        .into_any_element()
}

fn onboarding_summary_row(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    swatch: Option<u32>,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .text_color(rgb(roles.on_surface_variant))
                .child(label.into()),
        )
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .when_some(swatch, |this, color| {
                    this.child(div().size(space(10.0)).rounded_full().bg(rgb(color)))
                })
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(roles.on_surface))
                        .child(value.into()),
                ),
        )
        .into_any_element()
}

fn render_onboarding_security_step(
    step_index: usize,
    total_steps: usize,
    local_vault_status: LocalVaultStatus,
    settings: Entity<SettingsController>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let motion = cx.theme().motion_tokens().clone();
    let unlocked_t = transition(
        ("onboarding-security-status", "unlocked"),
        if local_vault_status == LocalVaultStatus::Unlocked {
            1.0
        } else {
            0.0
        },
        Transition::new(motion.duration_normal).easing(motion.easing_enter),
        window,
        cx,
    );
    let chip_bg = mix_hsla(
        rgb(roles.tertiary_container).into(),
        rgb(roles.primary_container).into(),
        unlocked_t,
    );
    let chip_fg = mix_hsla(
        rgb(roles.on_tertiary_container).into(),
        rgb(roles.on_primary_container).into(),
        unlocked_t,
    );

    let (description_key, action_key, status_key, status_icon, emblem_icon) =
        match local_vault_status {
            LocalVaultStatus::Disabled => (
                "onboarding.security.create_description",
                Some("onboarding.security.create_action"),
                "onboarding.security.status_setup_required",
                AppIcon::Lock,
                AppIcon::Vault,
            ),
            LocalVaultStatus::Locked => (
                "onboarding.security.unlock_description",
                Some("onboarding.security.unlock_action"),
                "onboarding.security.status_locked",
                AppIcon::Lock,
                AppIcon::Vault,
            ),
            LocalVaultStatus::Unlocked => (
                "onboarding.security.ready_description",
                None,
                "onboarding.security.status_ready",
                AppIcon::Check,
                AppIcon::Check,
            ),
        };

    v_flex()
        .w_full()
        .max_w(rems(38.0))
        .items_center()
        .gap_6()
        .child(m3_tonal_emblem(
            emblem_icon,
            116.0,
            76.0,
            36.0,
            EmptyStateTone::Primary,
        ))
        .child(render_onboarding_step_header(
            OnboardingStep::Security,
            step_index,
            total_steps,
            true,
        ))
        .child(onboarding_filled_surface(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap_5()
                .flex_wrap()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(rems(16.0))
                        .items_start()
                        .gap_3()
                        .child(onboarding_status_chip(
                            Some(status_icon),
                            i18n::string(status_key),
                            chip_bg,
                            chip_fg,
                        ))
                        .child(
                            div()
                                .w_full()
                                .text_size(miaominal_settings::FontSize::Heading.scaled())
                                .line_height(miaominal_settings::scaled_line_height(21.0))
                                .text_color(rgb(roles.on_surface_variant))
                                .child(i18n::string(description_key)),
                        ),
                )
                .when_some(action_key, |this, action_key| {
                    this.child(div().flex_shrink_0().child(onboarding_filled_button(
                        "onboarding-portable-vault-action",
                        i18n::string(action_key),
                        None,
                        false,
                        window,
                        cx,
                        move |window, cx| {
                            settings.update(cx, |controller, cx| {
                                controller.open_local_vault_passphrase_popup(
                                    LocalVaultPassphrasePopupMode::PrimaryAction,
                                    window,
                                    cx,
                                );
                            });
                        },
                    )))
                })
                .into_any_element(),
        ))
        .into_any_element()
}

fn sample_guarded_presence(
    seen_key: &'static str,
    presence_key: (&'static str, &'static str),
    present: bool,
    window: &mut Window,
    cx: &mut App,
) -> PresenceSample {
    let seen = window.use_keyed_state(seen_key, cx, |_, _| false);
    if present && !*seen.read(cx) {
        seen.update(cx, |flag, _| *flag = true);
    }
    if !*seen.read(cx) {
        return PresenceSample {
            phase: PresencePhase::Absent,
            progress: 0.0,
            status: MotionStatus::Idle,
        };
    }

    let motion = cx.theme().motion_tokens().clone();
    Presence::new(presence_key, present)
        .transition(Transition::new(motion.duration_fast).easing(if present {
            motion.easing_enter
        } else {
            motion.easing_exit
        }))
        .sample(window, cx)
}

#[allow(clippy::too_many_arguments)]
fn render_onboarding_footer(
    active_step: OnboardingStep,
    portable: bool,
    portable_vault_ready: bool,
    can_go_back: bool,
    can_advance: bool,
    can_finish: bool,
    settings: Entity<SettingsController>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let motion = cx.theme().motion_tokens().clone();
    let security_gate_blocked =
        portable && active_step == OnboardingStep::Security && !portable_vault_ready;
    let has_leading = can_go_back || security_gate_blocked;
    let solo_progress = spring(
        ("onboarding-footer", "solo"),
        if has_leading { 0.0 } else { 1.0 },
        motion.spring_move,
        window,
        cx,
    );
    let back_presence = sample_guarded_presence(
        "onboarding-footer-seen-back",
        ("onboarding-footer", "back"),
        can_go_back,
        window,
        cx,
    );
    let hint_presence = sample_guarded_presence(
        "onboarding-footer-seen-hint",
        ("onboarding-footer", "hint"),
        security_gate_blocked,
        window,
        cx,
    );
    let show_leading = back_presence.should_render() || hint_presence.should_render();
    let leading_progress = if can_go_back || !back_presence.should_render() {
        back_presence.progress.max(hint_presence.progress)
    } else {
        back_presence.progress
    };
    let overlay_leading = solo_progress > 0.001
        || matches!(back_presence.phase, PresencePhase::Exiting)
        || matches!(hint_presence.phase, PresencePhase::Exiting);

    let back_settings = settings.clone();
    let forward_settings = settings;

    let primary_button = if active_step == OnboardingStep::Finish {
        onboarding_filled_button(
            "onboarding-finish-enter-app",
            i18n::string("onboarding.navigation.finish"),
            Some(AppIcon::Forward),
            !can_finish,
            window,
            cx,
            move |_, cx| {
                forward_settings.update(cx, |controller, cx| {
                    controller.finish_onboarding(cx);
                });
            },
        )
    } else {
        let label_key = if active_step == OnboardingStep::Welcome {
            "onboarding.navigation.get_started"
        } else {
            "onboarding.navigation.next"
        };
        onboarding_filled_button(
            "onboarding-next",
            i18n::string(label_key),
            Some(AppIcon::Next),
            !can_advance,
            window,
            cx,
            move |_, cx| {
                forward_settings.update(cx, |controller, cx| {
                    controller.advance_onboarding_step(cx);
                });
            },
        )
    };

    let build_leading_group = || {
        h_flex()
            .items_center()
            .gap_3()
            .when(back_presence.should_render(), |this| {
                let back_id = if can_go_back {
                    "onboarding-back"
                } else {
                    "onboarding-back-exiting"
                };
                let back_settings = back_settings.clone();
                this.child(
                    div()
                        .relative()
                        .opacity(back_presence.progress)
                        .left(px(-10.0 * (1.0 - back_presence.progress)))
                        .child(
                            onboarding_quiet_tonal_button(
                                back_id,
                                i18n::string("onboarding.navigation.back"),
                                Some(AppIcon::ChevronLeft),
                                cx,
                                move |_, cx| {
                                    back_settings.update(cx, |controller, cx| {
                                        controller.previous_onboarding_step(cx);
                                    });
                                },
                            )
                            .disabled(!can_go_back),
                        ),
                )
            })
            .when(hint_presence.should_render(), |this| {
                this.child(
                    div()
                        .relative()
                        .opacity(hint_presence.progress)
                        .left(px(-6.0 * (1.0 - hint_presence.progress)))
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .text_color(rgb(roles.on_surface_variant))
                        .child(i18n::string("onboarding.navigation.security_gate_hint")),
                )
            })
    };

    let footer_row = if show_leading && !overlay_leading {
        h_flex()
            .w_full()
            .min_h(space(40.0))
            .items_center()
            .justify_between()
            .gap_3()
            .flex_wrap()
            .child(
                div()
                    .relative()
                    .opacity(leading_progress)
                    .child(build_leading_group()),
            )
            .child(primary_button)
            .into_any_element()
    } else {
        div()
            .relative()
            .w_full()
            .min_h(space(40.0))
            .flex()
            .items_center()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .child(div().flex_grow(1.0))
                    .child(primary_button)
                    .child(div().flex_grow(solo_progress)),
            )
            .when(show_leading, |this| {
                this.child(
                    div()
                        .id("onboarding-footer-leading-overlay")
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .child(build_leading_group()),
                )
            })
            .into_any_element()
    };

    div()
        .id("onboarding-footer")
        .test_support()
        .w_full()
        .flex_shrink_0()
        .rounded_b(space(28.0))
        .bg(rgb(roles.surface_container_low))
        .child(
            div()
                .w_full()
                .max_w(rems(48.0))
                .mx_auto()
                .px_8()
                .py_4()
                .child(footer_row),
        )
        .into_any_element()
}

fn mix_hsla(from: gpui_kit::Hsla, to: gpui_kit::Hsla, t: f32) -> gpui_kit::Hsla {
    let t = t.clamp(0.0, 1.0);
    let a = Rgba::from(from);
    let b = Rgba::from(to);
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
    .into()
}

fn onboarding_filled_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    trailing_icon: Option<AppIcon>,
    disabled: bool,
    window: &mut Window,
    cx: &mut App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let id: ElementId = id.into();
    let label = label.into();
    let roles = miaominal_settings::current_theme().material.roles;
    let motion = cx.theme().motion_tokens().clone();
    let enabled_t = transition(
        (id.clone(), "enabled"),
        if disabled { 0.0 } else { 1.0 },
        Transition::new(motion.duration_fast).easing(motion.easing_enter),
        window,
        cx,
    );
    let disabled_bg: gpui_kit::Hsla = color_with_alpha(roles.on_surface, 0x1E).into();
    let disabled_fg: gpui_kit::Hsla = color_with_alpha(roles.on_surface, 0x61).into();
    let enabled_bg: gpui_kit::Hsla = rgb(roles.primary).into();
    let enabled_fg: gpui_kit::Hsla = rgb(roles.on_primary).into();
    let background = mix_hsla(disabled_bg, enabled_bg, enabled_t);
    let foreground = mix_hsla(disabled_fg, enabled_fg, enabled_t);

    Button::new(id)
        .custom(
            ButtonCustomVariant::new(cx)
                .color(background)
                .foreground(foreground)
                .hover(if disabled {
                    background
                } else {
                    background.blend(foreground.opacity(0.08))
                })
                .active(if disabled {
                    background
                } else {
                    background.blend(foreground.opacity(0.14))
                }),
        )
        .large()
        .h(space(40.0))
        .px(space(24.0))
        .rounded(px(999.0))
        .border_0()
        .bg(background)
        .text_color(foreground)
        .font_weight(FontWeight::MEDIUM)
        .disabled(disabled)
        .accessibility_label(label.clone())
        .child(
            h_flex()
                .items_center()
                .gap(space(8.0))
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Heading.scaled())
                        .child(label),
                )
                .when_some(trailing_icon, |row, icon| {
                    row.child(Icon::new(icon).size(space(18.0)))
                }),
        )
        .on_click(move |_, window, cx| on_click(window, cx))
}

fn onboarding_tonal_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    leading_icon: Option<AppIcon>,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let colors = ShellColors::current();

    onboarding_custom_pill_button(
        id,
        label,
        leading_icon,
        colors.selected,
        colors.on_selected,
        cx,
        on_click,
    )
}

fn onboarding_quiet_tonal_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    leading_icon: Option<AppIcon>,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let roles = miaominal_settings::current_theme().material.roles;

    onboarding_custom_pill_button(
        id,
        label,
        leading_icon,
        rgb(roles.surface_container_high).into(),
        rgb(roles.on_surface).into(),
        cx,
        on_click,
    )
}

fn onboarding_custom_pill_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    leading_icon: Option<AppIcon>,
    background: gpui_kit::Hsla,
    foreground: gpui_kit::Hsla,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();

    Button::new(id)
        .custom(
            ButtonCustomVariant::new(cx)
                .color(background)
                .foreground(foreground)
                .hover(background.blend(foreground.opacity(0.08)))
                .active(background.blend(foreground.opacity(0.14))),
        )
        .large()
        .h(space(40.0))
        .px(space(20.0))
        .rounded(px(999.0))
        .border_0()
        .bg(background)
        .text_color(foreground)
        .font_weight(FontWeight::MEDIUM)
        .accessibility_label(label.clone())
        .child(
            h_flex()
                .items_center()
                .gap(space(8.0))
                .when_some(leading_icon, |row, icon| {
                    row.child(Icon::new(icon).size(space(18.0)))
                })
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Heading.scaled())
                        .child(label),
                ),
        )
        .on_click(move |_, window, cx| on_click(window, cx))
}

fn onboarding_status_chip(
    icon: Option<AppIcon>,
    label: impl Into<SharedString>,
    background: impl Into<gpui_kit::Hsla>,
    foreground: impl Into<gpui_kit::Hsla>,
) -> Div {
    h_flex()
        .flex_shrink_0()
        .items_center()
        .gap(space(6.0))
        .px(space(10.0))
        .py(space(4.0))
        .rounded_full()
        .bg(background.into())
        .text_color(foreground.into())
        .when_some(icon, |this, icon| {
            this.child(Icon::new(icon).size(space(14.0)))
        })
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .font_weight(FontWeight::MEDIUM)
                .child(label.into()),
        )
}

fn onboarding_assist_chip(text: impl Into<SharedString>) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    div()
        .flex_shrink_0()
        .px(space(14.0))
        .py(space(6.0))
        .rounded(space(16.0))
        .bg(rgb(roles.surface_container_highest))
        .text_color(rgb(roles.on_surface_variant))
        .text_size(miaominal_settings::FontSize::Input.scaled())
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
        .into_any_element()
}

fn onboarding_filled_surface(content: AnyElement) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    v_flex()
        .w_full()
        .min_w(px(0.0))
        .gap_4()
        .p_6()
        .rounded(space(28.0))
        .bg(rgb(roles.surface_container))
        .child(content)
        .into_any_element()
}

fn onboarding_panel_header(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> AnyElement {
    let title = title.into();
    let description = description.into();
    let roles = miaominal_settings::current_theme().material.roles;

    v_flex()
        .gap(space(6.0))
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Subtitle.scaled())
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(roles.on_surface))
                .child(title),
        )
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .line_height(miaominal_settings::scaled_line_height(18.0))
                .text_color(rgb(roles.on_surface_variant))
                .child(description),
        )
        .into_any_element()
}

fn onboarding_panel(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    content: AnyElement,
) -> AnyElement {
    onboarding_filled_surface(
        v_flex()
            .w_full()
            .gap_4()
            .child(onboarding_panel_header(title, description))
            .child(content)
            .into_any_element(),
    )
}

fn onboarding_category_card(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    background_color: u32,
    foreground_color: u32,
    content: AnyElement,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let title = title.into();
    let description = description.into();

    v_flex()
        .w_full()
        .min_w(px(0.0))
        .gap_4()
        .p_6()
        .rounded(space(28.0))
        .bg(rgb(background_color))
        .child(
            v_flex()
                .gap(space(4.0))
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Title.scaled())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(foreground_color))
                        .child(title),
                )
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .line_height(miaominal_settings::scaled_line_height(18.0))
                        .text_color(rgb(roles.on_surface_variant))
                        .child(description),
                ),
        )
        .child(content)
        .into_any_element()
}

fn onboarding_field(label: impl Into<SharedString>, content: AnyElement) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    v_flex()
        .min_w(rems(15.0))
        .flex_1()
        .gap_2()
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(roles.on_surface_variant))
                .child(label.into()),
        )
        .child(content)
        .into_any_element()
}

fn onboarding_field_with_description(
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    content: AnyElement,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let label = label.into();
    let description = description.into();

    v_flex()
        .min_w(rems(15.0))
        .flex_1()
        .gap_2()
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(roles.on_surface_variant))
                .child(label),
        )
        .child(content)
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .line_height(miaominal_settings::scaled_line_height(16.0))
                .text_color(rgb(roles.on_surface_variant))
                .child(description),
        )
        .into_any_element()
}

fn onboarding_theme_swatch(label_key: &'static str, color: u32) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    h_flex()
        .gap(space(6.0))
        .px(space(12.0))
        .py(space(6.0))
        .rounded_full()
        .bg(rgb(roles.surface_container_highest))
        .items_center()
        .child(div().size(space(10.0)).rounded_full().bg(rgb(color)))
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .text_color(rgb(roles.on_surface_variant))
                .child(i18n::string(label_key)),
        )
        .into_any_element()
}

fn step_label_key(step: OnboardingStep) -> &'static str {
    match step {
        OnboardingStep::Welcome => "onboarding.steps.welcome",
        OnboardingStep::Preferences => "onboarding.steps.preferences",
        OnboardingStep::Security => "onboarding.steps.security",
        OnboardingStep::Import => "onboarding.steps.import",
        OnboardingStep::Finish => "onboarding.steps.finish",
    }
}

fn step_title_key(step: OnboardingStep) -> &'static str {
    match step {
        OnboardingStep::Welcome => "onboarding.hero.title",
        OnboardingStep::Preferences => "onboarding.preferences.title",
        OnboardingStep::Security => "onboarding.security.title",
        OnboardingStep::Import => "onboarding.import.title",
        OnboardingStep::Finish => "onboarding.finish.title",
    }
}

fn step_subtitle_key(step: OnboardingStep) -> Option<&'static str> {
    match step {
        OnboardingStep::Welcome => Some("onboarding.hero.subtitle"),
        OnboardingStep::Preferences => Some("onboarding.preferences.subtitle"),
        OnboardingStep::Security => Some("onboarding.security.subtitle"),
        OnboardingStep::Import => Some("onboarding.import.subtitle"),
        OnboardingStep::Finish => None,
    }
}

fn onboarding_font_family_control(
    picker: Entity<FontFamilyPickerState>,
    entity: Entity<SettingsController>,
    cx: &App,
) -> AnyElement {
    setting_field_with_reset_action(
        font_family_picker(
            "onboarding-interface-font-family",
            &picker,
            Size::Medium,
            cx,
        ),
        false,
        move |window, cx| {
            entity.update(cx, |this, cx| {
                this.reset_font_family(window, cx);
            });
        },
    )
    .into_any_element()
}

fn onboarding_terminal_font_family_control(
    picker: Entity<FontFamilyPickerState>,
    entity: Entity<SettingsController>,
    cx: &App,
) -> AnyElement {
    setting_field_with_reset_action(
        font_family_picker("onboarding-terminal-font-family", &picker, Size::Medium, cx),
        false,
        move |window, cx| {
            entity.update(cx, |this, cx| {
                this.reset_terminal_font_family(window, cx);
            });
        },
    )
    .into_any_element()
}

fn onboarding_theme_control(
    current_theme: ThemeId,
    scale: f32,
    entity: Entity<SettingsController>,
) -> AnyElement {
    let selected_index = usize::from(current_theme == ThemeId::Dark);

    SegmentedSwitch::new("onboarding-theme-mode")
        .selected_index(selected_index)
        .width(204.0 * scale)
        .height(36.0 * scale)
        .padding(2.0 * scale)
        .item(i18n::string("enum.theme.light"))
        .item(i18n::string("enum.theme.dark"))
        .on_click(move |index, _, cx| {
            let theme = if index == 1 {
                ThemeId::Dark
            } else {
                ThemeId::Light
            };
            entity.update(cx, |this, cx| {
                this.set_theme(theme, cx);
            });
        })
        .into_any_element()
}

fn onboarding_font_fallbacks_control(
    input: Entity<InputState>,
    entity: Entity<SettingsController>,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    setting_field_with_reset_action(
        div().flex_1().min_w(px(0.0)).child(
            surface_text_input(&input, TextInputSurface::Highest)
                .with_size(Size::Medium)
                .text_color(rgb(roles.on_surface))
                .into_any_element(),
        ),
        false,
        move |window, cx| {
            entity.update(cx, |this, cx| {
                this.reset_font_fallbacks(window, cx);
            });
        },
    )
    .into_any_element()
}

fn onboarding_right_click_behavior_control(
    select_state: Entity<SelectState<Vec<SelectOption<TerminalRightClickBehavior>>>>,
) -> AnyElement {
    md3_select(&select_state)
        .with_size(Size::Medium)
        .w_full()
        .into_any_element()
}

fn onboarding_shift_right_click_control(
    enabled: bool,
    entity: Entity<SettingsController>,
) -> AnyElement {
    md3_switch("onboarding-shift-right-click-menu")
        .checked(enabled)
        .on_click(move |enabled, _, cx| {
            entity.update(cx, |this, cx| {
                this.set_terminal_shift_right_click_context_menu(*enabled, cx);
            });
        })
        .into_any_element()
}

fn onboarding_seed_color_control(
    current_seed_color: String,
    material: crate::ui::theme::MaterialTheme,
    picker: Entity<ColorPickerState>,
    entity: Entity<SettingsController>,
) -> AnyElement {
    let featured_colors = vec![
        rgb(material.source).into(),
        rgb(material.roles.primary).into(),
        rgb(material.roles.secondary).into(),
        rgb(material.roles.tertiary).into(),
        rgb(material.extended.success.color).into(),
        rgb(material.extended.warning.color).into(),
    ];

    v_flex()
        .w_full()
        .gap_3()
        .child(setting_field_with_reset_action(
            div().text_color(rgb(material.roles.on_surface)).child(
                ColorPicker::new(&picker)
                    .with_size(Size::Medium)
                    .label(current_seed_color)
                    .featured_colors(featured_colors),
            ),
            true,
            move |window, cx| {
                entity.update(cx, |this, cx| {
                    this.reset_seed_color(window, cx);
                });
            },
        ))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .flex_wrap()
                .child(onboarding_theme_swatch(
                    "settings.appearance.swatches.seed",
                    material.source,
                ))
                .child(onboarding_theme_swatch(
                    "settings.appearance.swatches.primary",
                    material.roles.primary,
                ))
                .child(onboarding_theme_swatch(
                    "settings.appearance.swatches.secondary",
                    material.roles.secondary,
                ))
                .child(onboarding_theme_swatch(
                    "settings.appearance.swatches.tertiary",
                    material.roles.tertiary,
                )),
        )
        .into_any_element()
}

fn onboarding_interface_font_size_stepper(
    value: String,
    entity: Entity<SettingsController>,
    cx: &App,
) -> AnyElement {
    let entity_for_dec = entity.clone();
    let entity_for_inc = entity;

    onboarding_number_stepper(
        "onboarding-interface-font-size",
        value,
        cx,
        move |_, cx| {
            entity_for_dec.update(cx, |this, cx| {
                this.adjust_interface_font_size(-miaominal_settings::STEP, cx);
            });
        },
        move |_, cx| {
            entity_for_inc.update(cx, |this, cx| {
                this.adjust_interface_font_size(miaominal_settings::STEP, cx);
            });
        },
    )
}

fn onboarding_terminal_font_size_stepper(
    value: String,
    entity: Entity<SettingsController>,
    cx: &App,
) -> AnyElement {
    let entity_for_dec = entity.clone();
    let entity_for_inc = entity;

    onboarding_number_stepper(
        "onboarding-terminal-font-size",
        value,
        cx,
        move |_, cx| {
            entity_for_dec.update(cx, |this, cx| {
                this.adjust_terminal_font_size(-miaominal_settings::STEP, cx);
            });
        },
        move |_, cx| {
            entity_for_inc.update(cx, |this, cx| {
                this.adjust_terminal_font_size(miaominal_settings::STEP, cx);
            });
        },
    )
}

fn onboarding_line_height_stepper(
    value: String,
    entity: Entity<SettingsController>,
    cx: &App,
) -> AnyElement {
    let entity_for_dec = entity.clone();
    let entity_for_inc = entity;

    onboarding_number_stepper(
        "onboarding-line-height",
        value,
        cx,
        move |_, cx| {
            entity_for_dec.update(cx, |this, cx| {
                this.adjust_line_height(-miaominal_settings::STEP, cx);
            });
        },
        move |_, cx| {
            entity_for_inc.update(cx, |this, cx| {
                this.adjust_line_height(miaominal_settings::STEP, cx);
            });
        },
    )
}

fn onboarding_number_stepper(
    id_prefix: &'static str,
    value: String,
    cx: &App,
    on_decrement: impl Fn(&mut Window, &mut App) + 'static,
    on_increment: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let hover = color_with_alpha(roles.on_surface, 0x14).into();
    let active = color_with_alpha(roles.on_surface, 0x1E).into();
    type StepButtonAction = Box<dyn Fn(&mut Window, &mut App)>;
    let step_button = |suffix: &'static str, label: &'static str, on_click: StepButtonAction| {
        Button::new(SharedString::from(format!("{id_prefix}-{suffix}")))
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(rgb(roles.surface_container_highest).into())
                    .foreground(rgb(roles.on_surface).into())
                    .hover(hover)
                    .active(active),
            )
            .size(space(32.0))
            .p_0()
            .rounded(px(999.0))
            .label(label)
            .on_click(move |_, window, cx| on_click(window, cx))
    };

    h_flex()
        .items_center()
        .gap_1()
        .p(space(4.0))
        .rounded(px(999.0))
        .bg(rgb(roles.surface_container_high))
        .child(step_button("dec", "-", Box::new(on_decrement)))
        .child(
            div()
                .min_w(rems(4.5))
                .px_2()
                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(roles.on_surface))
                .text_center()
                .child(value),
        )
        .child(step_button("inc", "+", Box::new(on_increment)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contracts::catalog::CatalogReader;
    use crate::ui::contracts::proxies::{ProxyCommitOperation, ProxyCommitter};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Bounds, Render, TestAppContext, WindowBounds, WindowOptions, size};
    use miaominal_core::profile::SessionProfile;
    use miaominal_core::proxy::ProxyProfile;
    use miaominal_secrets::{CredentialStore, LockedCredentialBackend, SecretStore};
    use miaominal_services::{OpenSshIntegrationService, SshBridgeService};
    use miaominal_settings::SshBridgeConfig;
    use miaominal_ssh::SshBridgeEndpoint;
    use miaominal_storage::{
        BridgeAuditLog, BridgeSecuritySettingsStore, KnownHostsStore, SettingsStore,
    };
    use miaominal_sync::{SyncConfig, SyncConfigStore, SyncEngine};

    struct OnboardingStepperHarness {
        active_step: OnboardingStep,
        steps: &'static [OnboardingStep],
        portable_vault_ready: bool,
        base_font: f32,
        selection_count: usize,
    }

    impl Render for OnboardingStepperHarness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            window.set_rem_size(px(self.base_font));
            let entity = cx.entity();

            div().size_full().child(
                div()
                    .id("onboarding-stepper")
                    .test_support()
                    .w_full()
                    .max_w(rems(48.0))
                    .mx_auto()
                    .px_4()
                    .py_3()
                    .child(build_onboarding_stepper(
                        self.active_step,
                        self.steps,
                        self.portable_vault_ready,
                        move |step, cx| {
                            entity.update(cx, |this, cx| {
                                let portable = this.steps.contains(&OnboardingStep::Security);
                                if matches!(
                                    onboarding_step_status(
                                        step,
                                        this.active_step,
                                        portable,
                                        this.portable_vault_ready,
                                    ),
                                    OnboardingStepStatus::Completed
                                        | OnboardingStepStatus::PendingReachable
                                ) {
                                    this.active_step = step;
                                    this.selection_count += 1;
                                    cx.notify();
                                }
                            });
                        },
                    )),
            )
        }
    }

    struct EmptyCatalog;
    impl CatalogReader for EmptyCatalog {
        fn profiles(&self) -> Vec<SessionProfile> {
            Vec::new()
        }
        fn profile(&self, _: &str) -> Option<SessionProfile> {
            None
        }
        fn proxies(&self) -> Vec<ProxyProfile> {
            Vec::new()
        }
    }

    struct UnusedProxyCommitter;
    impl ProxyCommitter for UnusedProxyCommitter {
        fn commit(
            &self,
            _: &mut App,
            _: ProxyCommitOperation,
        ) -> anyhow::Result<Vec<ProxyProfile>> {
            anyhow::bail!("unused in onboarding test")
        }
    }

    struct OnboardingPageHarness {
        settings: Entity<SettingsController>,
    }

    impl Render for OnboardingPageHarness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            render_onboarding_page(self.settings.clone(), None, window, cx)
        }
    }

    fn onboarding_test_settings_args(
        root: &std::path::Path,
        runtime: &tokio::runtime::Runtime,
        settings_store: SettingsStore,
        persistence: SettingsPersistencePort,
    ) -> SettingsControllerArgs {
        let bridge = SshBridgeService::new_with_stores(
            runtime.handle().clone(),
            SshBridgeEndpoint::derive(root).unwrap(),
            SshBridgeEndpoint::instance_id(root).unwrap(),
            root.join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(root.join("known_hosts")),
            BridgeSecuritySettingsStore::open(&root.join("settings.toml"))
                .map_err(|error| format!("{error:#}")),
            BridgeAuditLog::open(&root.join("bridge_audit.log"))
                .map_err(|error| format!("{error:#}")),
        );
        SettingsControllerArgs {
            runtime: runtime.handle().clone(),
            proxy_store: None,
            proxies: Vec::new(),
            settings_store,
            secrets: SecretStore::new_locked_vault(),
            sync_engine: SyncEngine {
                config_store: SyncConfigStore::with_credentials(
                    root.join("sync.toml"),
                    SyncConfig::default(),
                    CredentialStore::with_backend("onboarding-ui-test", LockedCredentialBackend),
                ),
                last_capability_report: None,
            },
            local_vault_status: LocalVaultStatus::Disabled,
            open_ssh_integration_service: OpenSshIntegrationService::new(
                bridge.clone(),
                root.join("ssh"),
                SshBridgeEndpoint::instance_id(root).unwrap(),
            ),
            ssh_bridge_service: bridge,
            sync_executor: None,
            auto_sync: miaominal_services::AutoSyncSnapshot {
                capability: Default::default(),
                capability_notice_id: None,
                updated_config: None,
                revision: 0,
                enabled: false,
                phase: miaominal_services::AutoSyncPhase::Disabled,
                message: None,
                last_result: None,
                last_result_id: None,
                dirty: false,
                retry_at_unix: None,
                intervention: None,
            },
            proxy_commit: ProxyCommitPort::new(UnusedProxyCommitter),
            auto_sync_control: AutoSyncControlPort::new(|_, _, _| false, |_| {}),
            persistence,
        }
    }

    #[test]
    fn onboarding_step_status_distinguishes_completed_current_and_blocked_steps() {
        assert_eq!(
            onboarding_step_status(
                OnboardingStep::Welcome,
                OnboardingStep::Preferences,
                false,
                false,
            ),
            OnboardingStepStatus::Completed
        );
        assert_eq!(
            onboarding_step_status(
                OnboardingStep::Preferences,
                OnboardingStep::Preferences,
                false,
                false,
            ),
            OnboardingStepStatus::Current
        );
        assert_eq!(
            onboarding_step_status(
                OnboardingStep::Import,
                OnboardingStep::Preferences,
                false,
                false,
            ),
            OnboardingStepStatus::PendingReachable
        );

        assert_eq!(
            onboarding_step_status(
                OnboardingStep::Welcome,
                OnboardingStep::Security,
                true,
                false,
            ),
            OnboardingStepStatus::Completed
        );
        assert_eq!(
            onboarding_step_status(
                OnboardingStep::Security,
                OnboardingStep::Security,
                true,
                false,
            ),
            OnboardingStepStatus::Current
        );
        assert_eq!(
            onboarding_step_status(
                OnboardingStep::Security,
                OnboardingStep::Welcome,
                true,
                false,
            ),
            OnboardingStepStatus::PendingReachable
        );
        assert_eq!(
            onboarding_step_status(OnboardingStep::Import, OnboardingStep::Welcome, true, false),
            OnboardingStepStatus::PendingBlocked
        );
        assert_eq!(
            onboarding_step_status(
                OnboardingStep::Finish,
                OnboardingStep::Security,
                true,
                false,
            ),
            OnboardingStepStatus::PendingBlocked
        );
        assert_eq!(
            onboarding_step_status(OnboardingStep::Import, OnboardingStep::Security, true, true),
            OnboardingStepStatus::PendingReachable
        );
    }

    #[gpui_kit::test]
    fn portable_stepper_blocks_steps_after_security_until_vault_is_ready(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            miaominal_settings::sync_component_theme(cx);
        });

        let steps = OnboardingStep::steps(true);
        let (handle, harness) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(960.0), px(640.0)),
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|_| OnboardingStepperHarness {
                        active_step: OnboardingStep::Welcome,
                        steps,
                        portable_vault_ready: false,
                        base_font: 14.0,
                        selection_count: 0,
                    })
                },
            )
            .expect("open onboarding stepper window")
        });

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("onboarding-stepper").visible());

            // Locked steps after Security reject clicks before the vault is ready.
            window.click("onboarding-step-import", cx);
            window.click("onboarding-step-finish", cx);
            assert_eq!(harness.read(cx).active_step, OnboardingStep::Welcome);
            assert_eq!(harness.read(cx).selection_count, 0);

            // Clicking current step is also a no-op.
            window.click("onboarding-step-welcome", cx);
            assert_eq!(harness.read(cx).active_step, OnboardingStep::Welcome);
            assert_eq!(harness.read(cx).selection_count, 0);

            // Reachable step (Security) can be selected directly.
            window.click("onboarding-step-security", cx);
            assert_eq!(harness.read(cx).active_step, OnboardingStep::Security);
            assert_eq!(harness.read(cx).selection_count, 1);

            // Steps after Security remain blocked while the vault is not unlocked.
            window.click("onboarding-step-import", cx);
            assert_eq!(harness.read(cx).active_step, OnboardingStep::Security);
            assert_eq!(harness.read(cx).selection_count, 1);

            // Completed steps remain clickable to navigate back.
            window.click("onboarding-step-welcome", cx);
            assert_eq!(harness.read(cx).active_step, OnboardingStep::Welcome);
            assert_eq!(harness.read(cx).selection_count, 2);

            // Unlocking the vault makes Import and Finish reachable.
            harness.update(cx, |this, cx| {
                this.portable_vault_ready = true;
                cx.notify();
            });
            window.render_frame(cx);

            window.click("onboarding-step-import", cx);
            assert_eq!(harness.read(cx).active_step, OnboardingStep::Import);
            window.click("onboarding-step-finish", cx);
            assert_eq!(harness.read(cx).active_step, OnboardingStep::Finish);
            window.click("onboarding-step-preferences", cx);
            assert_eq!(harness.read(cx).active_step, OnboardingStep::Preferences);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn onboarding_stepper_layout_adapts_cleanly_across_window_sizes(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            miaominal_settings::sync_component_theme(cx);
        });

        let steps = OnboardingStep::steps(true);
        for base_font in [14.0, 22.0] {
            for (width, height) in [(720.0, 480.0), (1040.0, 800.0)] {
                let (handle, _) = cx.update(|cx| {
                    gpui_kit::open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                                Default::default(),
                                size(px(width), px(height)),
                            ))),
                            ..Default::default()
                        },
                        cx,
                        |_, cx| {
                            cx.new(|_| OnboardingStepperHarness {
                                active_step: OnboardingStep::Security,
                                steps,
                                portable_vault_ready: false,
                                base_font,
                                selection_count: 0,
                            })
                        },
                    )
                    .expect("open layout test window")
                });

                cx.update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    let bounds: Vec<_> = steps
                        .iter()
                        .map(|step| {
                            let element = window.find(step_element_id(*step));
                            assert!(
                                element.visible(),
                                "step {:?} must be visible at {width}x{height} font={base_font}",
                                step
                            );
                            element.bounds()
                        })
                        .collect();

                    let first_top = f32::from(bounds[0].top());
                    for (ix, step_bounds) in bounds.iter().enumerate() {
                        assert!(
                            f32::from(step_bounds.left()) >= 0.0
                                && f32::from(step_bounds.right()) <= width + 1.0,
                            "horizontal step {ix} clipped outside {width}: {:?}",
                            step_bounds
                        );
                        assert!(
                            (f32::from(step_bounds.top()) - first_top).abs() <= 1.0,
                            "horizontal step {ix} misaligned vertically: top={:?} vs {first_top}",
                            step_bounds.top()
                        );
                        if ix > 0 {
                            assert!(
                                step_bounds.left() > bounds[ix - 1].right(),
                                "horizontal step {ix} overlaps or reorders previous step: {:?} vs {:?}",
                                bounds[ix - 1],
                                step_bounds
                            );
                        }
                    }
                })
                .unwrap();
            }
        }
    }

    #[gpui_kit::test]
    fn render_onboarding_page_drives_navigation_disclosure_and_responsive_chrome(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            miaominal_settings::sync_component_theme(cx);
        });

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("settings.toml");
        let store = SettingsStore::load_with_path(settings_path).unwrap();
        let args = onboarding_test_settings_args(
            directory.path(),
            &runtime,
            store,
            SettingsPersistencePort::new(|_| {}),
        );

        let (handle, harness) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(1040.0), px(920.0)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let settings = cx.new(|cx| {
                        SettingsController::new(
                            args,
                            CatalogQueryPort::new(Rc::new(EmptyCatalog)),
                            window,
                            cx,
                        )
                    });
                    cx.new(|_| OnboardingPageHarness { settings })
                },
            )
            .expect("open onboarding page test window")
        });
        let settings = cx.update(|cx| harness.read(cx).settings.clone());

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            cx.set_reduce_motion(true);
            assert!(window.find("onboarding-stepper-bar").visible());
            assert!(window.try_find("onboarding-rail").is_none());
            assert!(window.find("onboarding-footer").visible());
            assert!(window.try_find("onboarding-back").is_none());

            // Single footer button on Welcome must be centered horizontally.
            let footer_bounds = window.find("onboarding-footer").bounds();
            let welcome_next_bounds = window.find("onboarding-next").bounds();
            assert!(
                (f32::from(welcome_next_bounds.center().x) - f32::from(footer_bounds.center().x))
                    .abs()
                    <= 2.0,
                "single primary button in footer should be centered: next={:?}, footer={:?}",
                welcome_next_bounds,
                footer_bounds
            );

            assert_eq!(
                settings.read(cx).onboarding_state().onboarding_step,
                OnboardingStep::Welcome
            );

            window.click("onboarding-next", cx);
            assert_eq!(
                settings.read(cx).onboarding_state().onboarding_step,
                OnboardingStep::Preferences
            );
            assert!(window.find("onboarding-back").visible());

            // Expand progressive-disclosure advanced options inside Preferences and snap motion.
            cx.set_reduce_motion(true);
            window.render_frame(cx);
            let prefs_footer_bounds = window.find("onboarding-footer").bounds();
            let prefs_back_bounds = window.find("onboarding-back").bounds();
            let prefs_next_bounds = window.find("onboarding-next").bounds();
            assert!(prefs_back_bounds.center().x < prefs_footer_bounds.center().x);
            assert!(prefs_next_bounds.center().x > prefs_footer_bounds.center().x);

            window.click("onboarding-preferences-advanced-toggle", cx);

            window.click("onboarding-back", cx);
            assert_eq!(
                settings.read(cx).onboarding_state().onboarding_step,
                OnboardingStep::Welcome
            );
            window.render_frame(cx);
            assert!(window.try_find("onboarding-back").is_none());
            let back_welcome_next_bounds = window.find("onboarding-next").bounds();
            assert!(
                (f32::from(back_welcome_next_bounds.center().x)
                    - f32::from(window.find("onboarding-footer").bounds().center().x))
                .abs()
                    <= 2.0
            );

            window.click("onboarding-step-import", cx);
            assert_eq!(
                settings.read(cx).onboarding_state().onboarding_step,
                OnboardingStep::Import
            );

            window.click("onboarding-step-welcome", cx);
            assert_eq!(
                settings.read(cx).onboarding_state().onboarding_step,
                OnboardingStep::Welcome
            );
        })
        .unwrap();
    }
}
