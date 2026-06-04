use super::super::*;
use crate::ui::components::{
    editor_button_with_id, icon_button_with_icon_size, md3_select, md3_switch,
};
use crate::ui::i18n;
use gpui_component::breadcrumb::{Breadcrumb, BreadcrumbItem};
use miaominal_settings::{AppLanguage, TerminalRightClickBehavior, ThemeId};

const ONBOARDING_TITLE_BAR_HEIGHT: f32 = 56.0;
const ONBOARDING_STEP_TRANSITION_OFFSET: f32 = 8.0;
const ONBOARDING_TRAFFIC_LIGHT_PADDING: f32 = 71.0;

#[derive(Clone, Copy)]
struct OnboardingStepRenderState {
    step: OnboardingStep,
    visibility: f32,
}

fn onboarding_window_controls_on_left() -> bool {
    cfg!(target_os = "macos")
}

fn show_onboarding_macos_traffic_light_space(window: &Window) -> bool {
    cfg!(target_os = "macos") && !window.is_fullscreen()
}

impl AppView {
    pub(in crate::ui::shell) fn render_onboarding_page(
        &mut self,
        entity: Entity<AppView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let entity_finish = entity.clone();
        let entity_theme = entity.clone();
        let entity_font_family = entity.clone();
        let entity_font_fallbacks = entity.clone();
        let entity_font_size = entity.clone();
        let entity_line_height = entity.clone();
        let entity_shift_right_click = entity.clone();
        let step_render_state = self.onboarding_step_render_state(window);
        let current_step = step_render_state.step;
        let font_family_select = self.panel_forms.settings.font_family_select.clone();
        let font_fallbacks_input = self.panel_forms.settings.font_fallbacks_input.clone();
        let seed_color_picker = self.panel_forms.settings.seed_color_picker.clone();
        let terminal_right_click_behavior_select = self
            .panel_forms
            .settings
            .terminal_right_click_behavior_select
            .clone();
        let current_font_size = format!("{:.1}", self.settings_store.settings().font_size);
        let current_line_height = format!("{:.1}", self.settings_store.settings().line_height);
        let current_seed_color = self.settings_store.settings().seed_color.clone();
        let current_shift_right_click_context_menu = self
            .settings_store
            .settings()
            .terminal_shift_right_click_context_menu;
        let language_select = self.panel_forms.settings.language_select.clone();
        let current_theme = self.settings_store.settings().theme_id;
        let current_material =
            miaominal_settings::Theme::from_settings(self.settings_store.settings()).material;
        let desired_seed_color = rgb(current_material.source);

        if seed_color_picker.read(cx).value() != Some(desired_seed_color.into()) {
            seed_color_picker.update(cx, |picker, cx| {
                picker.set_value(desired_seed_color, window, cx);
            });
        }

        let step_content = match current_step {
            OnboardingStep::Welcome => {
                render_onboarding_welcome_step(entity.clone(), language_select.clone())
            }
            OnboardingStep::Preferences => render_onboarding_preferences_step(
                current_theme,
                current_seed_color,
                current_material,
                font_family_select,
                font_fallbacks_input,
                seed_color_picker,
                terminal_right_click_behavior_select,
                current_font_size,
                current_line_height,
                current_shift_right_click_context_menu,
                entity_theme,
                entity.clone(),
                entity_font_family,
                entity_font_fallbacks,
                entity_font_size,
                entity_line_height,
                entity_shift_right_click,
            ),
            OnboardingStep::Import => render_onboarding_import_step(self, entity.clone()),
            OnboardingStep::Finish => render_onboarding_finish_step(),
        };

        div()
            .size_full()
            .relative()
            .bg(rgb(roles.surface_container_low))
            .child(
                v_flex()
                    .size_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(render_onboarding_title_bar(
                        current_step,
                        entity.clone(),
                        window,
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .w_full()
                            .min_h(px(0.0))
                            .max_w(px(1040.0))
                            .mx_auto()
                            .px_4()
                            .pt_4()
                            .pb(px(112.0))
                            .gap_4()
                            .child(
                                v_flex()
                                    .relative()
                                    .items_center()
                                    .justify_center()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .gap_6()
                                    .p_6()
                                    .rounded(px(28.0))
                                    .opacity(step_render_state.visibility)
                                    .top(px((1.0 - step_render_state.visibility)
                                        * ONBOARDING_STEP_TRANSITION_OFFSET))
                                    .child(render_onboarding_step_header(current_step))
                                    .when(current_step == OnboardingStep::Welcome, |this| {
                                        this.child(
                                            div()
                                                .w_full()
                                                .flex()
                                                .justify_center()
                                                .text_color(rgb(roles.primary))
                                                .child(
                                                    Icon::new(AppIcon::Miaominal).size(px(220.0)),
                                                ),
                                        )
                                    })
                                    .child(step_content),
                            ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .right(px(0.0))
                    .bottom(px(32.0))
                    .left(px(0.0))
                    .child(render_onboarding_navigation(current_step, entity_finish)),
            )
            .when_some(
                Root::render_notification_layer(window, cx),
                |this, layer| this.child(layer),
            )
            .into_any_element()
    }

    fn onboarding_step_render_state(&mut self, window: &mut Window) -> OnboardingStepRenderState {
        let now = Instant::now();
        let desired_step = self.onboarding.onboarding_step;
        let duration = super::super::support::CONTAINER_TRANSITION_DURATION;

        if self.onboarding.visible_onboarding_step != desired_step {
            match self.onboarding.onboarding_step_transition {
                Some(transition) if transition.phase == OnboardingStepTransitionPhase::Exiting => {}
                _ => {
                    self.onboarding.onboarding_step_transition = Some(OnboardingStepTransition {
                        phase: OnboardingStepTransitionPhase::Exiting,
                        started_at: now,
                        duration,
                    });
                }
            }
        }

        if let Some(transition) = self.onboarding.onboarding_step_transition {
            let duration_seconds = transition.duration.as_secs_f32();
            if duration_seconds <= f32::EPSILON {
                self.onboarding.visible_onboarding_step = desired_step;
                self.onboarding.onboarding_step_transition = None;

                return OnboardingStepRenderState {
                    step: self.onboarding.visible_onboarding_step,
                    visibility: 1.0,
                };
            }

            let elapsed = now.saturating_duration_since(transition.started_at);
            let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
            let eased = progress * progress * (3.0 - 2.0 * progress);

            if progress >= 1.0 {
                match transition.phase {
                    OnboardingStepTransitionPhase::Exiting => {
                        self.onboarding.visible_onboarding_step = desired_step;
                        self.onboarding.onboarding_step_transition =
                            Some(OnboardingStepTransition {
                                phase: OnboardingStepTransitionPhase::Entering,
                                started_at: now,
                                duration: transition.duration,
                            });
                        window.request_animation_frame();

                        return OnboardingStepRenderState {
                            step: self.onboarding.visible_onboarding_step,
                            visibility: 0.0,
                        };
                    }
                    OnboardingStepTransitionPhase::Entering => {
                        self.onboarding.visible_onboarding_step = desired_step;
                        self.onboarding.onboarding_step_transition = None;

                        return OnboardingStepRenderState {
                            step: self.onboarding.visible_onboarding_step,
                            visibility: 1.0,
                        };
                    }
                }
            }

            window.request_animation_frame();

            return OnboardingStepRenderState {
                step: self.onboarding.visible_onboarding_step,
                visibility: match transition.phase {
                    OnboardingStepTransitionPhase::Exiting => 1.0 - eased,
                    OnboardingStepTransitionPhase::Entering => eased,
                },
            };
        }

        self.onboarding.visible_onboarding_step = desired_step;

        OnboardingStepRenderState {
            step: self.onboarding.visible_onboarding_step,
            visibility: 1.0,
        }
    }
}

fn render_onboarding_title_bar(
    step: OnboardingStep,
    entity: Entity<AppView>,
    window: &Window,
) -> AnyElement {
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
                .child(Icon::new(AppIcon::Miaominal).size(px(36.0)))
                .child(
                    v_flex()
                        .min_w(px(0.0))
                        .gap_1()
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(roles.on_surface))
                                .child(APP_TITLE),
                        )
                        .child(
                            div()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .child(render_onboarding_step_breadcrumb(step, entity)),
                        ),
                ),
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
                |window, _| {
                    window.remove_window();
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
                |window, _| {
                    window.remove_window();
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
        .rounded(px(16.0))
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

fn render_onboarding_step_breadcrumb(
    current_step: OnboardingStep,
    entity: Entity<AppView>,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let breadcrumb = OnboardingStep::ALL
        .into_iter()
        .fold(Breadcrumb::new(), |breadcrumb, step| {
            let is_current = step == current_step;
            let is_complete = step.index() < current_step.index();
            let foreground = if is_current || is_complete {
                roles.on_surface
            } else {
                roles.on_surface_variant
            };
            let item = if is_current {
                BreadcrumbItem::new(i18n::string(step_label_key(step)))
                    .disabled(true)
                    .text_color(rgb(foreground))
                    .font_weight(FontWeight::MEDIUM)
            } else {
                let entity = entity.clone();
                BreadcrumbItem::new(i18n::string(step_label_key(step)))
                    .text_color(rgb(foreground))
                    .font_weight(FontWeight::NORMAL)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.set_onboarding_step(step, cx);
                        });
                    })
            };

            breadcrumb.child(item)
        });

    div()
        .w_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .occlude()
        .when(
            cfg!(any(target_os = "linux", target_os = "freebsd")),
            |this| {
                this.on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .on_mouse_up(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
            },
        )
        .child(
            breadcrumb
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .line_height(miaominal_settings::scaled_line_height(14.0)),
        )
        .into_any_element()
}

fn render_onboarding_step_header(step: OnboardingStep) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    div()
        .w_full()
        .min_h(px(56.0))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Hero.scaled())
                .font_weight(FontWeight::BOLD)
                .text_center()
                .text_color(rgb(roles.on_surface))
                .child(i18n::string(step_title_key(step))),
        )
        .into_any_element()
}

fn render_onboarding_welcome_step(
    entity: Entity<AppView>,
    language_select: Entity<SelectState<Vec<SelectOption<AppLanguage>>>>,
) -> AnyElement {
    let _entity = entity;

    v_flex()
        .w_full()
        .items_center()
        .gap_4()
        .child(
            div().w_full().max_w(px(420.0)).child(onboarding_panel(
                i18n::string("settings.appearance.language.label"),
                i18n::string("settings.appearance.language.description"),
                md3_select(&language_select)
                    .with_size(gpui_component::Size::Medium)
                    .w_full()
                    .into_any_element(),
            )),
        )
        .into_any_element()
}

fn render_onboarding_import_step(app: &AppView, entity: Entity<AppView>) -> AnyElement {
    let entity_import = entity.clone();

    h_flex()
        .w_full()
        .justify_center()
        .child(
            div()
                .w_full()
                .max_w(px(520.0))
                .child(onboarding_filled_surface(
                    v_flex()
                        .w_full()
                        .gap_4()
                        .child(onboarding_panel_header(
                            i18n::string("settings.connections.import_source.label"),
                            i18n::string("settings.connections.import_action.description"),
                        ))
                        .child(
                            md3_select(&app.panel_forms.settings.profile_import_source_select)
                                .with_size(gpui_component::Size::Medium)
                                .w_full(),
                        )
                        .child(editor_button_with_id(
                            "onboarding-import",
                            i18n::string("settings.connections.import_action.action"),
                            false,
                            true,
                            false,
                            move |window, cx| {
                                entity_import.update(cx, |this, cx| {
                                    this.import_profiles_from_selected_source(window, cx);
                                });
                            },
                        ))
                        .child(
                            h_flex().gap_2().flex_wrap().children(
                                [
                                    i18n::string("settings.connections.import_sources.openssh"),
                                    i18n::string("settings.connections.import_sources.putty"),
                                    i18n::string("settings.connections.import_sources.securecrt"),
                                    i18n::string("settings.connections.import_sources.finalshell"),
                                ]
                                .into_iter()
                                .map(onboarding_tag),
                            ),
                        )
                        .into_any_element(),
                )),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_onboarding_preferences_step(
    current_theme: ThemeId,
    current_seed_color: String,
    current_material: crate::ui::theme::MaterialTheme,
    font_family_select: Entity<SelectState<SearchableVec<String>>>,
    font_fallbacks_input: Entity<InputState>,
    seed_color_picker: Entity<ColorPickerState>,
    terminal_right_click_behavior_select: Entity<
        SelectState<Vec<SelectOption<TerminalRightClickBehavior>>>,
    >,
    current_font_size: String,
    current_line_height: String,
    current_shift_right_click_context_menu: bool,
    entity_theme: Entity<AppView>,
    entity_seed_color: Entity<AppView>,
    entity_font_family: Entity<AppView>,
    entity_font_fallbacks: Entity<AppView>,
    entity_font_size: Entity<AppView>,
    entity_line_height: Entity<AppView>,
    entity_shift_right_click: Entity<AppView>,
) -> AnyElement {
    let section_background = current_material.roles.surface_container;
    let section_foreground = current_material.roles.on_surface;

    onboarding_surface(
        v_flex()
            .w_full()
            .gap_4()
            .child(
                h_flex()
                    .w_full()
                    .items_stretch()
                    .child(onboarding_category_card(
                        i18n::string("settings.appearance.typography_group.title"),
                        i18n::string("settings.appearance.typography_group.description"),
                        section_background,
                        section_foreground,
                        h_flex()
                            .w_full()
                            .gap_6()
                            .flex_wrap()
                            .items_start()
                            .child(
                                v_flex()
                                    .min_w(px(240.0))
                                    .flex_1()
                                    .gap_5()
                                    .child(onboarding_field(
                                        i18n::string("settings.appearance.font_family.label"),
                                        onboarding_font_family_control(
                                            font_family_select,
                                            entity_font_family,
                                        ),
                                    ))
                                    .child(onboarding_field_with_description(
                                        i18n::string("settings.appearance.font_fallbacks.label"),
                                        i18n::string(
                                            "settings.appearance.font_fallbacks.description",
                                        ),
                                        onboarding_font_fallbacks_control(
                                            font_fallbacks_input,
                                            entity_font_fallbacks,
                                        ),
                                    )),
                            )
                            .child(
                                v_flex()
                                    .min_w(px(220.0))
                                    .flex_1()
                                    .gap_5()
                                    .child(onboarding_field(
                                        i18n::string("settings.appearance.font_size.label"),
                                        onboarding_font_size_stepper(
                                            current_font_size,
                                            entity_font_size,
                                        ),
                                    ))
                                    .child(onboarding_field(
                                        i18n::string("settings.appearance.line_height.label"),
                                        onboarding_line_height_stepper(
                                            current_line_height,
                                            entity_line_height,
                                        ),
                                    )),
                            )
                            .into_any_element(),
                    )),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .flex_wrap()
                    .items_stretch()
                    .child(onboarding_category_card(
                        i18n::string("settings.appearance.theme_group.title"),
                        i18n::string("settings.appearance.theme_group.description"),
                        section_background,
                        section_foreground,
                        v_flex()
                            .w_full()
                            .gap_5()
                            .child(onboarding_field(
                                i18n::string("settings.appearance.dark_mode.label"),
                                onboarding_theme_control(current_theme, entity_theme.clone()),
                            ))
                            .child(onboarding_field(
                                i18n::string("settings.appearance.seed_color.label"),
                                onboarding_seed_color_control(
                                    current_seed_color,
                                    current_material,
                                    seed_color_picker,
                                    entity_seed_color,
                                ),
                            ))
                            .into_any_element(),
                    ))
                    .child(onboarding_category_card(
                        i18n::string("settings.key_bindings.mouse_group.title"),
                        i18n::string("settings.key_bindings.mouse_group.description"),
                        section_background,
                        section_foreground,
                        v_flex()
                            .w_full()
                            .gap_5()
                            .child(onboarding_field_with_description(
                                i18n::string("settings.key_bindings.right_click.label"),
                                i18n::string("settings.key_bindings.right_click.description"),
                                onboarding_right_click_behavior_control(
                                    terminal_right_click_behavior_select,
                                ),
                            ))
                            .child(onboarding_field_with_description(
                                i18n::string("settings.key_bindings.shift_right_click.label"),
                                i18n::string("settings.key_bindings.shift_right_click.description"),
                                onboarding_shift_right_click_control(
                                    current_shift_right_click_context_menu,
                                    entity_shift_right_click,
                                ),
                            ))
                            .into_any_element(),
                    )),
            )
            .into_any_element(),
    )
}

fn render_onboarding_finish_step() -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    v_flex()
        .w_full()
        .gap_8()
        .items_center()
        .justify_center()
        .child(
            h_flex()
                .gap_5()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(AppIcon::Sparkles)
                        .size(px(48.0))
                        .text_color(rgb(roles.primary)),
                )
                .child(Icon::new(AppIcon::Miaominal).size(px(140.0)))
                .child(
                    Icon::new(AppIcon::Sparkles)
                        .size(px(48.0))
                        .text_color(rgb(roles.primary)),
                ),
        )
        .child(
            v_flex().gap_3().items_center().child(
                div()
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .text_color(rgb(roles.on_surface_variant))
                    .text_center()
                    .max_w(px(420.0))
                    .child(i18n::string(
                        "onboarding.finish.congratulations_description",
                    )),
            ),
        )
        .into_any_element()
}

fn render_onboarding_navigation(
    step: OnboardingStep,
    entity_finish: Entity<AppView>,
) -> AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_center()
        .gap_3()
        .flex_wrap()
        .child(if step == OnboardingStep::Finish {
            onboarding_navigation_button(
                "onboarding-finish-enter-app",
                AppIcon::Forward,
                move |_, cx| {
                    entity_finish.update(cx, |this, cx| {
                        this.finish_onboarding(cx);
                    });
                },
            )
        } else {
            onboarding_navigation_button("onboarding-next", AppIcon::Next, move |_, cx| {
                entity_finish.update(cx, |this, cx| {
                    this.advance_onboarding_step(cx);
                });
            })
            .into_any_element()
        })
        .into_any_element()
}

fn onboarding_navigation_button(
    id: &'static str,
    icon: AppIcon,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    div()
        .id(SharedString::from(id))
        .child(icon_button_with_icon_size(
            icon,
            24.0,
            crate::ui::components::IconButtonStyle {
                size: 56.0,
                corner_radius: 99.0,
                background: Some(roles.primary),
                foreground: Some(roles.on_primary),
                border: None,
            },
            on_click,
        ))
        .into_any_element()
}

fn onboarding_surface(content: AnyElement) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w(px(280.0))
        .max_w_full()
        .gap_4()
        .p_5()
        .rounded(px(24.0))
        //.bg(rgb(roles.surface_container_high))
        .child(content)
        .into_any_element()
}

fn onboarding_filled_surface(content: AnyElement) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    v_flex()
        .flex_1()
        .min_w(px(280.0))
        .max_w_full()
        .gap_4()
        .p_5()
        .rounded(px(24.0))
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
        .gap_2()
        .child(
            div()
                .text_size(miaominal_settings::FontSize::SectionTitle.scaled())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(roles.on_surface))
                .child(title),
        )
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                .line_height(miaominal_settings::scaled_line_height(20.0))
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
    let title = title.into();
    let description = description.into();

    v_flex()
        .flex_1()
        .min_w(px(280.0))
        .gap_4()
        .p_5()
        .rounded(px(22.0))
        .bg(rgb(background_color))
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Subtitle.scaled())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(foreground_color))
                        .child(title),
                )
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .line_height(miaominal_settings::scaled_line_height(18.0))
                        .text_color(rgb(foreground_color))
                        .opacity(0.82)
                        .child(description),
                ),
        )
        .child(content)
        .into_any_element()
}

fn onboarding_field(label: impl Into<SharedString>, content: AnyElement) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    v_flex()
        .min_w(px(220.0))
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
        .min_w(px(220.0))
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

fn onboarding_tag(text: impl Into<SharedString>) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    pill_label(
        text,
        roles.surface_container_highest,
        roles.on_surface_variant,
    )
    .into_any_element()
}

fn onboarding_theme_swatch(label_key: &'static str, color: u32) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    h_flex()
        .gap_2()
        .items_center()
        .child(div().size(px(12.0)).rounded(px(999.0)).bg(rgb(color)))
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
        OnboardingStep::Import => "onboarding.steps.import",
        OnboardingStep::Finish => "onboarding.steps.finish",
    }
}

fn step_title_key(step: OnboardingStep) -> &'static str {
    match step {
        OnboardingStep::Welcome => "onboarding.hero.title",
        OnboardingStep::Preferences => "onboarding.preferences.title",
        OnboardingStep::Import => "onboarding.import.title",
        OnboardingStep::Finish => "onboarding.finish.title",
    }
}

fn onboarding_font_family_control(
    select_state: Entity<SelectState<SearchableVec<String>>>,
    entity: Entity<AppView>,
) -> AnyElement {
    setting_field_with_reset_action(
        md3_select(&select_state)
            .with_size(gpui_component::Size::Medium)
            .w_full()
            .into_any_element(),
        false,
        move |window, cx| {
            entity.update(cx, |this, cx| {
                this.reset_font_family(window, cx);
            });
        },
    )
    .into_any_element()
}

fn onboarding_theme_control(current_theme: ThemeId, entity: Entity<AppView>) -> AnyElement {
    md3_switch("onboarding-theme-mode")
        .checked(current_theme == ThemeId::Dark)
        .on_click(move |enabled, _, cx| {
            let theme = if *enabled {
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
    entity: Entity<AppView>,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    setting_field_with_reset_action(
        div().flex_1().min_w(px(0.0)).child(
            surface_text_input(&input, TextInputSurface::Highest)
                .with_size(gpui_component::Size::Medium)
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
        .with_size(gpui_component::Size::Medium)
        .w_full()
        .into_any_element()
}

fn onboarding_shift_right_click_control(enabled: bool, entity: Entity<AppView>) -> AnyElement {
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
    entity: Entity<AppView>,
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
                    .with_size(gpui_component::Size::Medium)
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
                .gap_4()
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

fn onboarding_font_size_stepper(value: String, entity: Entity<AppView>) -> AnyElement {
    let entity_for_dec = entity.clone();
    let entity_for_inc = entity;

    onboarding_stepper(
        "onboarding-font-size",
        value,
        move |_, cx| {
            entity_for_dec.update(cx, |this, cx| {
                this.adjust_font_size(-miaominal_settings::STEP, cx);
            });
        },
        move |_, cx| {
            entity_for_inc.update(cx, |this, cx| {
                this.adjust_font_size(miaominal_settings::STEP, cx);
            });
        },
    )
}

fn onboarding_line_height_stepper(value: String, entity: Entity<AppView>) -> AnyElement {
    let entity_for_dec = entity.clone();
    let entity_for_inc = entity;

    onboarding_stepper(
        "onboarding-line-height",
        value,
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

fn onboarding_stepper(
    id_prefix: &'static str,
    value: String,
    on_decrement: impl Fn(&mut Window, &mut App) + 'static,
    on_increment: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    h_flex()
        .gap_2()
        .items_center()
        .child(editor_button_with_id(
            SharedString::from(format!("{id_prefix}-dec")),
            "-",
            false,
            false,
            false,
            move |window, cx| on_decrement(window, cx),
        ))
        .child(
            div()
                .min_w(px(84.0))
                .px_3()
                .py_2()
                .rounded(px(10.0))
                .bg(rgb(roles.surface_container_highest))
                .text_color(rgb(roles.on_surface))
                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                .text_center()
                .child(value),
        )
        .child(editor_button_with_id(
            SharedString::from(format!("{id_prefix}-inc")),
            "+",
            false,
            false,
            false,
            move |window, cx| on_increment(window, cx),
        ))
        .into_any_element()
}
