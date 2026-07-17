use super::super::super::*;
use crate::ui::assets::AppIcon;
use crate::ui::components::{editor_button_with_id, md3_select, md3_spinner};
use crate::ui::i18n;
use gpui::{Axis, KeyDownEvent};
use gpui_component::{
    Disableable, Icon, Size,
    group_box::GroupBoxVariant,
    setting::{
        RenderOptions, SettingField, SettingFieldElement, SettingGroup, SettingItem, SettingPage,
        Settings,
    },
};
use miaominal_settings::{self, KeyBinding, ThemeId};
use miaominal_sync::SyncProvider;

pub(in crate::ui::shell) fn render_settings_page(
    settings: Entity<SettingsController>,
) -> gpui::AnyElement {
    Settings::new("app-settings")
        .with_size(Size::Large)
        .with_group_variant(GroupBoxVariant::Outline)
        .sidebar_width(px(220.0))
        .pages(setting_pages(settings))
        .into_any_element()
}

fn setting_pages(settings: Entity<SettingsController>) -> Vec<SettingPage> {
    vec![
        appearance_page(settings.clone()),
        connections_page(settings.clone()),
        key_bindings_page(settings.clone()),
        ai_providers_page(settings.clone()),
        sync_page(settings.clone()),
        vault_page(settings.clone()),
        about_page(settings),
    ]
}

fn appearance_page(entity: Entity<SettingsController>) -> SettingPage {
    let default_font_family = miaominal_settings::default_font_family();
    let font_size_min = format!("{:.1}", miaominal_settings::FONT_SIZE_MIN);
    let font_size_max = format!("{:.1}", miaominal_settings::FONT_SIZE_MAX);
    let line_height_min = format!("{:.1}", miaominal_settings::LINE_HEIGHT_MIN);
    let line_height_max = format!("{:.1}", miaominal_settings::LINE_HEIGHT_MAX);

    SettingPage::new(i18n::string("settings.pages.appearance.title"))
        .description(i18n::string("settings.pages.appearance.description"))
        .default_open(true)
        .resettable(false)
        .groups(vec![
            SettingGroup::new()
                .title(i18n::string("settings.appearance.language_group.title"))
                .description(i18n::string(
                    "settings.appearance.language_group.description",
                ))
                .item(
                    SettingItem::new(
                        i18n::string("settings.appearance.language.label"),
                        SettingField::element(AppLanguageField::new(entity.clone())),
                    )
                    .description(i18n::string("settings.appearance.language.description")),
                ),
            SettingGroup::new()
                .title(i18n::string("settings.appearance.typography_group.title"))
                .description(i18n::string(
                    "settings.appearance.typography_group.description",
                ))
                .items(vec![
                    SettingItem::new(
                        i18n::string("settings.appearance.font_family.label"),
                        SettingField::element(FontFamilyField::new(entity.clone())),
                    )
                    .layout(Axis::Vertical)
                    .description(i18n::string_args(
                        "settings.appearance.font_family.description",
                        &[("font", &default_font_family)],
                    )),
                    SettingItem::new(
                        i18n::string("settings.appearance.font_fallbacks.label"),
                        SettingField::element(FontFallbacksField::new(entity.clone())),
                    )
                    .layout(Axis::Vertical)
                    .description(i18n::string(
                        "settings.appearance.font_fallbacks.description",
                    )),
                    SettingItem::new(
                        i18n::string("settings.appearance.font_size.label"),
                        SettingField::render({
                            let entity = entity.clone();
                            move |options, _, cx| {
                                let size = options.size;
                                let id_prefix = SharedString::from(format!(
                                    "settings-font-size-{}-{}-{}",
                                    options.page_ix, options.group_ix, options.item_ix
                                ));
                                render_font_size_stepper(entity.clone(), id_prefix, size, cx)
                            }
                        }),
                    )
                    .description(i18n::string_args(
                        "settings.appearance.font_size.description",
                        &[("min", &font_size_min), ("max", &font_size_max)],
                    )),
                    SettingItem::new(
                        i18n::string("settings.appearance.line_height.label"),
                        SettingField::render({
                            let entity = entity.clone();
                            move |options, _, cx| {
                                let size = options.size;
                                let id_prefix = SharedString::from(format!(
                                    "settings-line-height-{}-{}-{}",
                                    options.page_ix, options.group_ix, options.item_ix
                                ));
                                render_line_height_stepper(entity.clone(), id_prefix, size, cx)
                            }
                        }),
                    )
                    .description(i18n::string_args(
                        "settings.appearance.line_height.description",
                        &[("min", &line_height_min), ("max", &line_height_max)],
                    )),
                ]),
            SettingGroup::new()
                .title(i18n::string("settings.appearance.theme_group.title"))
                .description(i18n::string("settings.appearance.theme_group.description"))
                .items(vec![
                    SettingItem::new(
                        i18n::string("settings.appearance.dark_mode.label"),
                        SettingField::switch(
                            {
                                let entity = entity.clone();
                                move |cx: &App| entity.read(cx).settings().theme_id == ThemeId::Dark
                            },
                            {
                                let entity = entity.clone();
                                move |enabled: bool, cx: &mut App| {
                                    entity.update(cx, |this, cx| {
                                        let theme_id = if enabled {
                                            ThemeId::Dark
                                        } else {
                                            ThemeId::Light
                                        };
                                        this.set_theme(theme_id, cx);
                                    });
                                }
                            },
                        ),
                    )
                    .description(i18n::string("settings.appearance.dark_mode.description")),
                    SettingItem::new(
                        i18n::string("settings.appearance.seed_color.label"),
                        SettingField::element(SeedColorField::new(entity.clone())),
                    )
                    .layout(Axis::Vertical)
                    .description(i18n::string("settings.appearance.seed_color.description")),
                ]),
        ])
}

fn connections_page(settings: Entity<SettingsController>) -> SettingPage {
    SettingPage::new(i18n::string("settings.pages.connections.title"))
        .description(i18n::string("settings.pages.connections.description"))
        .resettable(false)
        .groups(vec![
            SettingGroup::new()
                .title(i18n::string("settings.connections.recent_group.title"))
                .description(i18n::string(
                    "settings.connections.recent_group.description",
                ))
                .items(vec![
                    SettingItem::new(
                        i18n::string("settings.connections.recent_count.label"),
                        SettingField::render({
                            let entity = settings.clone();
                            move |options, _, cx| {
                                let size = options.size;
                                let id_prefix = SharedString::from(format!(
                                    "settings-recent-count-{}-{}-{}",
                                    options.page_ix, options.group_ix, options.item_ix
                                ));
                                render_recent_connections_stepper(
                                    entity.clone(),
                                    id_prefix,
                                    size,
                                    cx,
                                )
                            }
                        }),
                    )
                    .description(i18n::string_args(
                        "settings.connections.recent_count.description",
                        &[(
                            "max",
                            &miaominal_settings::RECENT_CONNECTIONS_COUNT_MAX.to_string(),
                        )],
                    )),
                ]),
            SettingGroup::new()
                .title(i18n::string("settings.connections.monitor_group.title"))
                .description(i18n::string(
                    "settings.connections.monitor_group.description",
                ))
                .items(vec![
                    SettingItem::new(
                        i18n::string("settings.connections.auto_collect_monitoring.label"),
                        SettingField::switch(
                            {
                                let entity = settings.clone();
                                move |cx: &App| {
                                    entity.read(cx).settings().auto_collect_session_monitoring
                                }
                            },
                            {
                                let entity = settings.clone();
                                move |enabled: bool, cx: &mut App| {
                                    entity.update(cx, |controller, cx| {
                                        controller.emit(
                                            AppCommand::SessionMonitoringPreferenceChanged(enabled),
                                            cx,
                                        );
                                    });
                                }
                            },
                        ),
                    )
                    .description(i18n::string(
                        "settings.connections.auto_collect_monitoring.description",
                    )),
                    SettingItem::new(
                        i18n::string("settings.connections.monitor_history.label"),
                        SettingField::element(MonitorHistoryDurationField::new(settings.clone())),
                    )
                    .description(i18n::string(
                        "settings.connections.monitor_history.description",
                    )),
                ]),
            SettingGroup::new()
                .title(i18n::string("settings.connections.tabs_group.title"))
                .description(i18n::string("settings.connections.tabs_group.description"))
                .items(vec![
                    SettingItem::new(
                        i18n::string("settings.connections.last_tab_close_behavior.label"),
                        SettingField::element(LastTabCloseBehaviorField::new(settings.clone())),
                    )
                    .description(i18n::string(
                        "settings.connections.last_tab_close_behavior.description",
                    )),
                ]),
            SettingGroup::new()
                .title(i18n::string("settings.connections.import_group.title"))
                .description(i18n::string(
                    "settings.connections.import_group.description",
                ))
                .items(vec![
                    SettingItem::new(
                        i18n::string("settings.connections.import_source.label"),
                        SettingField::element(ProfileImportSourceField::new(settings.clone())),
                    )
                    .description(i18n::string(
                        "settings.connections.import_source.description",
                    )),
                    SettingItem::new(
                        i18n::string("settings.connections.import_action.label"),
                        SettingField::element(ProfileImportActionField::new(settings.clone())),
                    )
                    .description(i18n::string(
                        "settings.connections.import_action.description",
                    )),
                ]),
        ])
}

fn about_page(settings: Entity<SettingsController>) -> SettingPage {
    let settings_reset_local = settings.clone();

    SettingPage::new(i18n::string("settings.pages.about.title"))
        .description(i18n::string("settings.pages.about.description"))
        .resettable(false)
        .groups(vec![
            SettingGroup::new()
                .title(i18n::string("settings.about.overview.title"))
                .item(SettingItem::render(|_, _, _| {
                    let material = miaominal_settings::current_theme().material;
                    let roles = material.roles;
                    let text_muted = crate::ui::theme::palette_tone_rgb(
                        material.palettes.neutral_variant,
                        if material.dark { 65 } else { 50 },
                    );

                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .justify_center()
                                .text_color(rgb(roles.primary))
                                .child(Icon::from(AppIcon::Miaominal).size(px(96.0))),
                        )
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Title.scaled())
                                .text_color(rgb(roles.on_surface))
                                .child("Miaominal"),
                        )
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(i18n::string_args(
                                    "settings.about.overview.version",
                                    &[("version", env!("CARGO_PKG_VERSION"))],
                                )),
                        )
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Input.scaled())
                                .text_color(rgb(text_muted))
                                .child(i18n::string("settings.about.overview.description")),
                        )
                        .into_any_element()
                })),
            SettingGroup::new()
                .title(i18n::string("settings.about.onboarding.title"))
                .item(
                    SettingItem::new(
                        i18n::string("settings.about.onboarding.label"),
                        SettingField::element(OnboardingActionField::new(settings)),
                    )
                    .description(i18n::string("settings.about.onboarding.description")),
                ),
            SettingGroup::new()
                .title(i18n::string("settings.about.reset_local.title"))
                .item(
                    SettingItem::new(
                        i18n::string("settings.about.reset_local.label"),
                        SettingField::element(ResetLocalDataActionField::new(settings_reset_local)),
                    )
                    .description(i18n::string("settings.about.reset_local.description")),
                ),
        ])
}

#[derive(Clone)]
struct AppLanguageField {
    controller: Entity<SettingsController>,
}

impl AppLanguageField {
    fn new(controller: Entity<SettingsController>) -> Self {
        Self { controller }
    }
}

impl SettingFieldElement for AppLanguageField {
    type Element = AnyElement;

    fn render_field(
        &self,
        options: &RenderOptions,
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        let select_state = self.controller.read(cx).forms.language_select.clone();

        md3_select(&select_state)
            .with_size(options.size)
            .w_full()
            .into_any_element()
    }
}

#[derive(Clone)]
struct MonitorHistoryDurationField {
    controller: Entity<SettingsController>,
}

impl MonitorHistoryDurationField {
    fn new(controller: Entity<SettingsController>) -> Self {
        Self { controller }
    }
}

impl SettingFieldElement for MonitorHistoryDurationField {
    type Element = AnyElement;

    fn render_field(
        &self,
        options: &RenderOptions,
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        let select_state = self
            .controller
            .read(cx)
            .forms
            .monitor_history_select
            .clone();

        md3_select(&select_state)
            .with_size(options.size)
            .w_full()
            .into_any_element()
    }
}

#[derive(Clone)]
struct LocalVaultAutoLockDurationField {
    controller: Entity<SettingsController>,
}

impl LocalVaultAutoLockDurationField {
    fn new(controller: Entity<SettingsController>) -> Self {
        Self { controller }
    }
}

impl SettingFieldElement for LocalVaultAutoLockDurationField {
    type Element = AnyElement;

    fn render_field(
        &self,
        options: &RenderOptions,
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        let select_state = self
            .controller
            .read(cx)
            .forms
            .local_vault_auto_lock_duration_select
            .clone();

        md3_select(&select_state)
            .with_size(options.size)
            .w_full()
            .into_any_element()
    }
}

#[derive(Clone)]
struct LastTabCloseBehaviorField {
    controller: Entity<SettingsController>,
}

impl LastTabCloseBehaviorField {
    fn new(controller: Entity<SettingsController>) -> Self {
        Self { controller }
    }
}

impl SettingFieldElement for LastTabCloseBehaviorField {
    type Element = AnyElement;

    fn render_field(
        &self,
        options: &RenderOptions,
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        let select_state = self
            .controller
            .read(cx)
            .forms
            .last_tab_close_behavior_select
            .clone();

        md3_select(&select_state)
            .with_size(options.size)
            .w_full()
            .into_any_element()
    }
}

#[derive(Clone)]
struct ProfileImportSourceField {
    controller: Entity<SettingsController>,
}

impl ProfileImportSourceField {
    fn new(controller: Entity<SettingsController>) -> Self {
        Self { controller }
    }
}

impl SettingFieldElement for ProfileImportSourceField {
    type Element = AnyElement;

    fn render_field(
        &self,
        options: &RenderOptions,
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        let select_state = self
            .controller
            .read(cx)
            .forms
            .profile_import_source_select
            .clone();

        md3_select(&select_state)
            .with_size(options.size)
            .w_full()
            .into_any_element()
    }
}

#[derive(Clone)]
struct ProfileImportActionField {
    controller: Entity<SettingsController>,
}

impl ProfileImportActionField {
    fn new(controller: Entity<SettingsController>) -> Self {
        Self { controller }
    }
}

impl SettingFieldElement for ProfileImportActionField {
    type Element = Button;

    fn render_field(
        &self,
        _options: &RenderOptions,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::Element {
        let controller = self.controller.clone();

        editor_button_with_id(
            "settings-import-profiles",
            i18n::string("settings.connections.import_action.action"),
            false,
            true,
            false,
            move |_window, cx| {
                controller.update(cx, |controller, cx| {
                    controller.request_profile_import(cx);
                });
            },
        )
    }
}

#[derive(Clone)]
struct FontFamilyField {
    entity: Entity<SettingsController>,
}

impl FontFamilyField {
    fn new(entity: Entity<SettingsController>) -> Self {
        Self { entity }
    }
}

impl SettingFieldElement for FontFamilyField {
    type Element = AnyElement;

    fn render_field(&self, options: &RenderOptions, _: &mut Window, cx: &mut App) -> Self::Element {
        let select_state = self.entity.read(cx).forms.font_family_select.clone();
        let entity = self.entity.clone();

        setting_field_with_reset_action(
            md3_select(&select_state)
                .with_size(options.size)
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
}

#[derive(Clone)]
struct FontFallbacksField {
    entity: Entity<SettingsController>,
}

impl FontFallbacksField {
    fn new(entity: Entity<SettingsController>) -> Self {
        Self { entity }
    }
}

impl SettingFieldElement for FontFallbacksField {
    type Element = AnyElement;

    fn render_field(&self, options: &RenderOptions, _: &mut Window, cx: &mut App) -> Self::Element {
        let roles = miaominal_settings::current_theme().material.roles;
        let input = self.entity.read(cx).forms.font_fallbacks_input.clone();
        let entity = self.entity.clone();

        setting_field_with_reset_action(
            div().flex_1().min_w(px(0.0)).child(
                surface_text_input(&input, TextInputSurface::Highest)
                    .with_size(options.size)
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
}

#[derive(Clone)]
struct SeedColorField {
    entity: Entity<SettingsController>,
}

impl SeedColorField {
    fn new(entity: Entity<SettingsController>) -> Self {
        Self { entity }
    }
}

impl SettingFieldElement for SeedColorField {
    type Element = AnyElement;

    fn render_field(
        &self,
        options: &RenderOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        let current = self.entity.read(cx).settings().clone();
        let picker = self.entity.read(cx).forms.seed_color_picker.clone();
        let entity = self.entity.clone();
        let material = miaominal_settings::Theme::from_settings(&current).material;
        let desired_color = rgb(material.source);

        if picker.read(cx).value() != Some(desired_color.into()) {
            picker.update(cx, |picker, cx| {
                picker.set_value(desired_color, window, cx);
            });
        }

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
                        .with_size(options.size)
                        .label(current.seed_color.clone())
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
                    .child(theme_swatch(
                        "settings.appearance.swatches.seed",
                        material.source,
                    ))
                    .child(theme_swatch(
                        "settings.appearance.swatches.primary",
                        material.roles.primary,
                    ))
                    .child(theme_swatch(
                        "settings.appearance.swatches.secondary",
                        material.roles.secondary,
                    ))
                    .child(theme_swatch(
                        "settings.appearance.swatches.tertiary",
                        material.roles.tertiary,
                    )),
            )
            .into_any_element()
    }
}

#[derive(Clone)]
struct OnboardingActionField {
    controller: Entity<SettingsController>,
}

impl OnboardingActionField {
    fn new(controller: Entity<SettingsController>) -> Self {
        Self { controller }
    }
}

impl SettingFieldElement for OnboardingActionField {
    type Element = Button;

    fn render_field(&self, _options: &RenderOptions, _: &mut Window, _: &mut App) -> Self::Element {
        let controller = self.controller.clone();

        settings_action_button(
            "settings-show-onboarding",
            i18n::string("settings.about.onboarding.action"),
            false,
            move |_, cx| {
                controller.update(cx, |controller, cx| {
                    controller.open_onboarding(cx);
                });
            },
        )
    }
}

#[derive(Clone)]
struct ResetLocalDataActionField {
    controller: Entity<SettingsController>,
}

impl ResetLocalDataActionField {
    fn new(controller: Entity<SettingsController>) -> Self {
        Self { controller }
    }
}

impl SettingFieldElement for ResetLocalDataActionField {
    type Element = Button;

    fn render_field(
        &self,
        _options: &RenderOptions,
        _: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        let controller = self.controller.clone();
        let disabled = controller.read(cx).local_data_reset_in_progress();

        editor_button_with_id(
            "settings-reset-local-data",
            i18n::string("settings.about.reset_local.action"),
            false,
            true,
            disabled,
            move |_, cx| {
                controller.update(cx, |controller, cx| {
                    if !controller.local_data_reset_in_progress() {
                        controller
                            .set_local_data_reset_confirm(Some(PendingLocalDataResetConfirmState));
                        cx.notify();
                    }
                });
            },
        )
    }
}

fn settings_action_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    primary: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    editor_button_with_id(id, label, primary, true, false, on_click)
}

fn settings_compact_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    editor_button_with_id(id, label, false, false, false, on_click)
}

fn render_recent_connections_stepper(
    entity: Entity<SettingsController>,
    id_prefix: SharedString,
    size: Size,
    cx: &App,
) -> AnyElement {
    let count = entity.read(cx).settings().recent_connections_count;
    let value = if count == 0 {
        i18n::string("settings.values.off")
    } else {
        count.to_string()
    };
    let entity_for_dec = entity.clone();
    let entity_for_inc = entity;

    stepper_control(
        id_prefix,
        value,
        size,
        move |_, cx| {
            let entity = entity_for_dec.clone();
            entity.update(cx, |this, cx| {
                this.adjust_recent_connections_count(-1, cx);
            });
        },
        move |_, cx| {
            let entity = entity_for_inc.clone();
            entity.update(cx, |this, cx| {
                this.adjust_recent_connections_count(1, cx);
            });
        },
    )
    .into_any_element()
}

fn render_font_size_stepper(
    entity: Entity<SettingsController>,
    id_prefix: SharedString,
    size: Size,
    cx: &App,
) -> AnyElement {
    let value = format!("{:.1}", entity.read(cx).settings().font_size);
    let entity_for_dec = entity.clone();
    let entity_for_inc = entity;

    stepper_control(
        id_prefix,
        value,
        size,
        move |_, cx| {
            let entity = entity_for_dec.clone();
            entity.update(cx, |this, cx| {
                this.adjust_font_size(-miaominal_settings::STEP, cx);
            });
        },
        move |_, cx| {
            let entity = entity_for_inc.clone();
            entity.update(cx, |this, cx| {
                this.adjust_font_size(miaominal_settings::STEP, cx);
            });
        },
    )
    .into_any_element()
}

fn render_line_height_stepper(
    entity: Entity<SettingsController>,
    id_prefix: SharedString,
    size: Size,
    cx: &App,
) -> AnyElement {
    let value = format!("{:.1}", entity.read(cx).settings().line_height);
    let entity_for_dec = entity.clone();
    let entity_for_inc = entity;

    stepper_control(
        id_prefix,
        value,
        size,
        move |_, cx| {
            let entity = entity_for_dec.clone();
            entity.update(cx, |this, cx| {
                this.adjust_line_height(-miaominal_settings::STEP, cx);
            });
        },
        move |_, cx| {
            let entity = entity_for_inc.clone();
            entity.update(cx, |this, cx| {
                this.adjust_line_height(miaominal_settings::STEP, cx);
            });
        },
    )
    .into_any_element()
}

fn stepper_control(
    id_prefix: SharedString,
    value: String,
    _size: Size,
    on_decrement: impl Fn(&mut Window, &mut App) + 'static,
    on_increment: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;

    h_flex()
        .gap_2()
        .items_center()
        .child(settings_compact_button(
            SharedString::from(format!("{}-dec", id_prefix)),
            "-",
            move |window, cx| on_decrement(window, cx),
        ))
        .child(
            div()
                .min_w(px(84.0))
                .px_3()
                .py_2()
                .rounded(px(10.0))
                .bg(rgb(roles.surface_container_high))
                .text_color(rgb(roles.on_surface))
                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                .text_center()
                .child(value),
        )
        .child(settings_compact_button(
            SharedString::from(format!("{}-inc", id_prefix)),
            "+",
            move |window, cx| on_increment(window, cx),
        ))
}

fn theme_swatch(label_key: &'static str, color: u32) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;

    h_flex()
        .gap_2()
        .items_center()
        .child(div().size(px(14.0)).rounded_full().bg(rgb(color)))
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .text_color(rgb(roles.on_surface_variant))
                .child(i18n::string(label_key)),
        )
}

fn ai_providers_page(settings: Entity<SettingsController>) -> SettingPage {
    SettingPage::new(i18n::string("settings.pages.ai_providers.title"))
        .description(i18n::string("settings.pages.ai_providers.description"))
        .resettable(false)
        .groups(vec![
            SettingGroup::new()
                .title(i18n::string("settings.ai_providers.providers_group.title"))
                .description(i18n::string(
                    "settings.ai_providers.providers_group.description",
                ))
                .item(
                    SettingItem::new(
                        i18n::string("settings.ai_providers.saved.label"),
                        SettingField::render({
                            let settings = settings.clone();
                            move |options, _, cx| {
                                render_ai_provider_selector(settings.clone(), options.size, cx)
                            }
                        }),
                    )
                    .layout(Axis::Vertical)
                    .description(i18n::string("settings.ai_providers.saved.description")),
                ),
            web_search_settings_group(settings),
        ])
}

fn web_search_settings_group(settings: Entity<SettingsController>) -> SettingGroup {
    SettingGroup::new()
        .title(i18n::string("settings.web_search.group.title"))
        .description(i18n::string("settings.web_search.group.description"))
        .items(vec![
            SettingItem::new(
                i18n::string("settings.web_search.enabled.label"),
                SettingField::switch(
                    {
                        let settings = settings.clone();
                        move |cx: &App| settings.read(cx).settings().web_search.enabled
                    },
                    {
                        let settings = settings.clone();
                        move |enabled: bool, cx: &mut App| {
                            settings.update(cx, |controller, cx| {
                                controller.set_web_search_enabled(enabled, cx);
                            });
                        }
                    },
                ),
            )
            .description(i18n::string("settings.web_search.enabled.description")),
            SettingItem::new(
                i18n::string("settings.web_search.kind.label"),
                SettingField::render({
                    let settings = settings.clone();
                    move |_, _, cx| render_web_search_config_field(settings.clone(), cx)
                }),
            )
            .description(i18n::string("settings.web_search.kind.description")),
        ])
}

fn render_web_search_config_field(settings: Entity<SettingsController>, cx: &App) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let web_search = settings.read(cx).settings().web_search.clone();
    let provider_label = i18n::string(web_search_provider_kind_label_key(web_search.kind));
    let save_in_progress = settings.read(cx).web_search_save_in_progress();
    let settings_edit = settings.clone();

    h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(
            div()
                .flex_1()
                .min_w(px(220.0))
                .px_3()
                .py_2()
                .rounded(px(10.0))
                .bg(rgb(roles.surface_container_high))
                .text_color(rgb(roles.on_surface))
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .child(provider_label),
        )
        .child(if save_in_progress {
            div()
                .id("web-search-config-entry-spinner")
                .size(px(34.0))
                .rounded(px(12.0))
                .flex()
                .items_center()
                .justify_center()
                .child(md3_spinner(16.0))
                .into_any_element()
        } else {
            icon_button(
                AppIcon::Edit,
                34.0,
                12.0,
                None,
                None,
                Some(roles.outline_variant),
                move |window, cx| {
                    settings_edit.update(cx, |controller, cx| {
                        controller.open_web_search_config_popup(window, cx);
                    });
                },
            )
            .into_any_element()
        })
        .into_any_element()
}

fn render_ai_provider_selector(
    settings: Entity<SettingsController>,
    size: Size,
    cx: &App,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let select = settings.read(cx).forms().ai_provider_select;
    let selected_provider_id = settings.read(cx).selected_ai_provider_id(cx);
    let settings_new = settings.clone();
    let settings_edit = settings.clone();

    v_flex()
        .w_full()
        .gap_3()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(260.0))
                        .child(md3_select(&select).with_size(size).w_full()),
                )
                .child(icon_button(
                    AppIcon::Plus,
                    34.0,
                    12.0,
                    None,
                    None,
                    Some(roles.outline_variant),
                    move |window, cx| {
                        settings_new.update(cx, |controller, cx| {
                            controller.start_new_ai_provider(window, cx);
                        });
                    },
                ))
                .child(if selected_provider_id.is_some() {
                    icon_button(
                        AppIcon::Edit,
                        34.0,
                        12.0,
                        None,
                        None,
                        Some(roles.outline_variant),
                        move |window, cx| {
                            settings_edit.update(cx, |controller, cx| {
                                if let Some(provider_id) = controller.selected_ai_provider_id(cx) {
                                    controller.edit_ai_provider(provider_id, window, cx);
                                }
                            });
                        },
                    )
                    .into_any_element()
                } else {
                    div()
                        .size(px(34.0))
                        .rounded(px(12.0))
                        .bg(rgb(roles.surface_container_low))
                        .border_color(rgb(roles.outline_variant))
                        .opacity(0.45)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(roles.on_surface_variant))
                        .child(Icon::new(AppIcon::Edit).small())
                        .into_any_element()
                }),
        )
        .into_any_element()
}

fn key_bindings_page(entity: Entity<SettingsController>) -> SettingPage {
    let workspace_slots = [
        KeyBindingSlot::NextTab,
        KeyBindingSlot::CloseTab,
        KeyBindingSlot::ReopenTab,
        KeyBindingSlot::OpenSettings,
    ];
    let terminal_slots = [
        KeyBindingSlot::Copy,
        KeyBindingSlot::Paste,
        KeyBindingSlot::Search,
        KeyBindingSlot::SplitRight,
        KeyBindingSlot::SplitDown,
        KeyBindingSlot::ClosePane,
    ];

    let workspace_items = key_binding_items(entity.clone(), &workspace_slots);
    let terminal_items = key_binding_items(entity.clone(), &terminal_slots);

    SettingPage::new(i18n::string("settings.pages.key_bindings.title"))
        .description(i18n::string("settings.pages.key_bindings.description"))
        .resettable(false)
        .groups(vec![
            SettingGroup::new()
                .title(i18n::string("settings.key_bindings.workspace_group.title"))
                .description(i18n::string(
                    "settings.key_bindings.workspace_group.description",
                ))
                .items(workspace_items),
            SettingGroup::new()
                .title(i18n::string("settings.key_bindings.terminal_group.title"))
                .description(i18n::string(
                    "settings.key_bindings.terminal_group.description",
                ))
                .items(terminal_items),
            SettingGroup::new()
                .title(i18n::string("settings.key_bindings.mouse_group.title"))
                .description(i18n::string(
                    "settings.key_bindings.mouse_group.description",
                ))
                .items(vec![
                    SettingItem::new(
                        i18n::string("settings.key_bindings.right_click.label"),
                        SettingField::element(TerminalRightClickBehaviorField::new(entity.clone())),
                    )
                    .description(i18n::string(
                        "settings.key_bindings.right_click.description",
                    )),
                    SettingItem::new(
                        i18n::string("settings.key_bindings.shift_right_click.label"),
                        SettingField::switch(
                            {
                                let entity = entity.clone();
                                move |cx: &App| {
                                    entity
                                        .read(cx)
                                        .settings()
                                        .terminal_shift_right_click_context_menu
                                }
                            },
                            {
                                let entity = entity.clone();
                                move |enabled: bool, cx: &mut App| {
                                    entity.update(cx, |this, cx| {
                                        this.set_terminal_shift_right_click_context_menu(
                                            enabled, cx,
                                        );
                                    });
                                }
                            },
                        ),
                    )
                    .description(i18n::string(
                        "settings.key_bindings.shift_right_click.description",
                    )),
                ]),
        ])
}

fn key_binding_items(
    entity: Entity<SettingsController>,
    slots: &[KeyBindingSlot],
) -> Vec<SettingItem> {
    slots
        .iter()
        .copied()
        .map(|slot| {
            SettingItem::new(
                slot.label(),
                SettingField::element(KeyBindingCaptureField::new(entity.clone(), slot)),
            )
            .description(slot.description())
        })
        .collect()
}

#[derive(Clone)]
struct TerminalRightClickBehaviorField {
    controller: Entity<SettingsController>,
}

impl TerminalRightClickBehaviorField {
    fn new(controller: Entity<SettingsController>) -> Self {
        Self { controller }
    }
}

impl SettingFieldElement for TerminalRightClickBehaviorField {
    type Element = AnyElement;

    fn render_field(
        &self,
        options: &RenderOptions,
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        let select_state = self
            .controller
            .read(cx)
            .forms
            .terminal_right_click_behavior_select
            .clone();

        md3_select(&select_state)
            .with_size(options.size)
            .w_full()
            .into_any_element()
    }
}

#[derive(Clone)]
struct SyncProviderField {
    controller: Entity<SettingsController>,
}

impl SyncProviderField {
    fn new(controller: Entity<SettingsController>) -> Self {
        Self { controller }
    }
}

impl SettingFieldElement for SyncProviderField {
    type Element = AnyElement;

    fn render_field(
        &self,
        options: &RenderOptions,
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        let select_state = self.controller.read(cx).forms().sync_provider_select;
        let selected_provider = select_state
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(self.controller.read(cx).sync_config().provider);
        let roles = miaominal_settings::current_theme().material.roles;
        let controller = self.controller.clone();

        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w(px(240.0))
                    .child(md3_select(&select_state).with_size(options.size).w_full()),
            )
            .child(if selected_provider == SyncProvider::None {
                div()
                    .size(px(34.0))
                    .rounded(px(12.0))
                    .bg(rgb(roles.surface_container_low))
                    .border_color(rgb(roles.outline_variant))
                    .opacity(0.45)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(roles.on_surface_variant))
                    .child(Icon::new(AppIcon::Edit).small())
                    .into_any_element()
            } else {
                icon_button(
                    AppIcon::Edit,
                    34.0,
                    12.0,
                    None,
                    None,
                    Some(roles.outline_variant),
                    move |window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.open_selected_sync_provider_config_popup(window, cx);
                        });
                    },
                )
                .into_any_element()
            })
            .into_any_element()
    }
}

#[derive(Clone)]
struct KeyBindingCaptureField {
    controller: Entity<SettingsController>,
    slot: KeyBindingSlot,
}

impl KeyBindingCaptureField {
    fn new(controller: Entity<SettingsController>, slot: KeyBindingSlot) -> Self {
        Self { controller, slot }
    }
}

impl SettingFieldElement for KeyBindingCaptureField {
    type Element = AnyElement;

    fn render_field(
        &self,
        options: &RenderOptions,
        _window: &mut Window,
        cx: &mut App,
    ) -> Self::Element {
        let settings_controller = self.controller.read(cx);
        let settings = settings_controller.settings().clone();
        let is_recording = settings_controller.recording_binding() == Some(self.slot);
        let capture_focus = settings_controller.forms.key_capture_focus.clone();
        let pending_preview = settings_controller.pending_preview().map(str::to_owned);
        let has_pending_binding = settings_controller.pending_binding().is_some();
        let roles = miaominal_settings::current_theme().material.roles;

        let current_binding = match self.slot {
            KeyBindingSlot::NextTab => settings.key_bindings.next_tab.display(),
            KeyBindingSlot::CloseTab => settings.key_bindings.close_tab.display(),
            KeyBindingSlot::ReopenTab => settings.key_bindings.reopen_tab.display(),
            KeyBindingSlot::OpenSettings => settings.key_bindings.open_settings.display(),
            KeyBindingSlot::Copy => settings.key_bindings.copy.display(),
            KeyBindingSlot::Paste => settings.key_bindings.paste.display(),
            KeyBindingSlot::Search => settings.key_bindings.search.display(),
            KeyBindingSlot::SplitRight => settings.key_bindings.split_right.display(),
            KeyBindingSlot::SplitDown => settings.key_bindings.split_down.display(),
            KeyBindingSlot::ClosePane => settings.key_bindings.close_pane.display(),
        };

        let controller = self.controller.clone();
        let slot = self.slot;
        let controller_for_change = self.controller.clone();
        let controller_for_reset = self.controller.clone();

        h_flex()
            .w_full()
            .gap_3()
            .items_center()
            .when(is_recording, |this| {
                let controller_capture = controller.clone();
                let preview_text: SharedString = match &pending_preview {
                    Some(p) => SharedString::from(p.clone()),
                    None => i18n::string("settings.key_bindings.capture.press_combo").into(),
                };
                // Show a different bg when a valid binding is staged
                let capture_bg = if has_pending_binding {
                    roles.secondary_container
                } else {
                    roles.primary_container
                };
                let capture_fg = if has_pending_binding {
                    roles.on_secondary_container
                } else {
                    roles.on_primary_container
                };
                let controller_save = controller.clone();
                let controller_cancel = controller.clone();
                let roles_save = roles;
                this.child(
                    v_flex()
                        .flex_1()
                        .gap_1()
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "key-capture-{}-{}-{}",
                                    options.page_ix, options.group_ix, options.item_ix
                                )))
                                .track_focus(&capture_focus)
                                .w_full()
                                .px_3()
                                .py_2()
                                .rounded(px(14.0))
                                .bg(rgb(capture_bg))
                                .text_color(rgb(capture_fg))
                                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(preview_text)
                                .on_key_down(move |event: &KeyDownEvent, _window, cx| {
                                    let ks = &event.keystroke;
                                    let key = ks.key.as_str();
                                    let is_modifier_only = matches!(
                                        key,
                                        "control" | "alt" | "shift" | "meta" | "super"
                                    );
                                    let has_required_modifier =
                                        ks.modifiers.control || ks.modifiers.alt;
                                    let binding = if has_required_modifier && !is_modifier_only {
                                        Some(KeyBinding::new(
                                            ks.modifiers.control,
                                            ks.modifiers.shift,
                                            ks.modifiers.alt,
                                            ks.key.to_lowercase(),
                                        ))
                                    } else {
                                        None
                                    };
                                    let preview = format_keystroke_preview(ks);
                                    controller_capture.update(cx, |this, cx| {
                                        this.update_key_preview(preview, binding, cx);
                                    });
                                }),
                        )
                        .child(
                            div()
                                .text_size(miaominal_settings::FontSize::Body.scaled())
                                .text_color(rgb(roles.on_surface_variant))
                                .child(SharedString::from(i18n::string(
                                    "settings.key_bindings.capture.esc_to_cancel",
                                ))),
                        ),
                )
                .when(has_pending_binding, |this| {
                    this.child(icon_button(
                        AppIcon::Check,
                        28.0,
                        8.0,
                        Some(roles_save.secondary_container),
                        Some(roles_save.on_secondary_container),
                        None,
                        move |_window, cx| {
                            controller_save.update(cx, |this, cx| {
                                this.accept_pending_key_binding(cx);
                            });
                        },
                    ))
                })
                .child(icon_button(
                    AppIcon::Close,
                    28.0,
                    8.0,
                    None,
                    None,
                    None,
                    move |_window, cx| {
                        controller_cancel.update(cx, |this, cx| {
                            this.cancel_recording_key_binding(cx);
                        });
                    },
                ))
            })
            .when(!is_recording, |this| {
                this.child(
                    pill_label(
                        current_binding,
                        roles.surface_container_high,
                        roles.on_surface,
                    )
                    .py_1()
                    .rounded(px(10.0))
                    .text_size(miaominal_settings::FontSize::Input.scaled())
                    .font_weight(gpui::FontWeight::MEDIUM),
                )
            })
            .when(!is_recording, |this| {
                this.child(icon_button(
                    AppIcon::Edit,
                    28.0,
                    8.0,
                    None,
                    None,
                    None,
                    move |window, cx| {
                        controller_for_change.update(cx, |this, cx| {
                            this.begin_recording_key_binding(slot, window, cx);
                        });
                    },
                ))
            })
            .when(!is_recording, |this| {
                this.child(icon_button(
                    AppIcon::Rotate,
                    28.0,
                    8.0,
                    None,
                    None,
                    None,
                    move |_window, cx| {
                        controller_for_reset.update(cx, |this, cx| {
                            this.reset_key_binding(slot, cx);
                        });
                    },
                ))
            })
            .into_any_element()
    }
}

fn format_keystroke_preview(ks: &gpui::Keystroke) -> String {
    let mut parts = String::new();
    if ks.modifiers.control {
        parts.push_str(&i18n::string("settings.key_bindings.capture.modifier.ctrl"));
    }
    if ks.modifiers.alt {
        parts.push_str(&i18n::string("settings.key_bindings.capture.modifier.alt"));
    }
    if ks.modifiers.shift {
        parts.push_str(&i18n::string(
            "settings.key_bindings.capture.modifier.shift",
        ));
    }
    let key_upper = ks.key.to_uppercase();
    if matches!(
        key_upper.as_str(),
        "CONTROL" | "ALT" | "SHIFT" | "META" | "SUPER"
    ) {
        parts.push('-');
    } else {
        parts.push_str(&key_upper);
    }
    parts
}

fn sync_page(settings: Entity<SettingsController>) -> SettingPage {
    SettingPage::new(i18n::string("settings.pages.sync.title"))
        .description(i18n::string("settings.pages.sync.description"))
        .resettable(false)
        .groups(vec![
            sync_status_group(settings.clone()),
            sync_encryption_group(settings.clone()),
            sync_provider_group(settings),
        ])
}

fn vault_page(settings: Entity<SettingsController>) -> SettingPage {
    SettingPage::new(i18n::string("settings.pages.vault.title"))
        .description(i18n::string("settings.pages.vault.description"))
        .resettable(false)
        .groups(vec![sync_vault_group(settings)])
}

fn sync_vault_group(settings: Entity<SettingsController>) -> SettingGroup {
    SettingGroup::new()
        .title(i18n::string("settings.sync.vault_group.title"))
        .description(i18n::string("settings.sync.vault_group.description"))
        .items(vec![
            SettingItem::new(
                i18n::string("settings.sync.vault.status.label"),
                SettingField::render({
                    let settings = settings.clone();
                    move |_, _, cx| {
                        let controller = settings.read(cx);
                        let disable_in_progress = controller.local_vault_disable_in_progress();
                        let status_text = controller.local_vault_status_label();
                        let show_lock =
                            controller.local_vault_status() == LocalVaultStatus::Unlocked;
                        let action = show_lock.then(|| {
                            let settings = settings.clone();
                            settings_action_button(
                                "local-vault-lock",
                                i18n::string("settings.sync.vault.actions.lock"),
                                false,
                                move |_window, cx| {
                                    settings.update(cx, |controller, cx| {
                                        controller.emit(
                                            AppCommand::VaultActionRequested(
                                                LocalVaultActionRequest::Lock,
                                            ),
                                            cx,
                                        );
                                    });
                                },
                            )
                            .disabled(disable_in_progress)
                        });
                        render_text_action_field(status_text, action)
                    }
                }),
            )
            .description(i18n::string("settings.sync.vault.status.description")),
            SettingItem::new(
                i18n::string("settings.sync.vault.passphrase.label"),
                SettingField::render({
                    let settings = settings.clone();
                    move |_, _, cx| {
                        let controller = settings.read(cx);
                        let disable_in_progress = controller.local_vault_disable_in_progress();
                        let needs_passphrase = controller.local_vault_requires_passphrase();
                        let can_change_passphrase = controller.local_vault_can_change_passphrase();
                        if needs_passphrase || can_change_passphrase {
                            let popup_mode = if needs_passphrase {
                                LocalVaultPassphrasePopupMode::PrimaryAction
                            } else {
                                LocalVaultPassphrasePopupMode::ChangePassphrase
                            };
                            let action_label =
                                controller.local_vault_passphrase_popup_title(popup_mode);
                            let settings = settings.clone();
                            render_supporting_action_field(
                                settings_action_button(
                                    "local-vault-passphrase-sheet",
                                    action_label,
                                    false,
                                    move |window, cx| {
                                        settings.update(cx, |controller, cx| {
                                            controller.open_local_vault_passphrase_popup(
                                                popup_mode, window, cx,
                                            );
                                        });
                                    },
                                )
                                .disabled(disable_in_progress),
                            )
                        } else {
                            render_text_action_field(
                                i18n::string("settings.sync.vault.passphrase.unlocked_hint"),
                                None,
                            )
                        }
                    }
                }),
            )
            .description(i18n::string("settings.sync.vault.passphrase.description")),
            SettingItem::new(
                i18n::string("settings.sync.vault.auto_lock_duration.label"),
                SettingField::element(LocalVaultAutoLockDurationField::new(settings.clone())),
            )
            .description(i18n::string(
                "settings.sync.vault.auto_lock_duration.description",
            )),
            SettingItem::new(
                i18n::string("settings.sync.vault.disable.label"),
                SettingField::render({
                    let settings = settings.clone();
                    move |_, _, cx| {
                        let controller = settings.read(cx);
                        let disable_in_progress = controller.local_vault_disable_in_progress();
                        if disable_in_progress {
                            v_flex()
                                .w_full()
                                .gap_3()
                                .child(
                                    h_flex().w_full().justify_end().child(
                                        div()
                                            .id("local-vault-disable-spinner")
                                            .min_w(px(116.0))
                                            .min_h(px(32.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(md3_spinner(18.0)),
                                    ),
                                )
                                .into_any_element()
                        } else if controller.local_vault_can_disable() {
                            let settings = settings.clone();
                            settings_action_button(
                                "local-vault-disable",
                                i18n::string("settings.sync.vault.actions.disable"),
                                false,
                                move |window, cx| {
                                    settings.update(cx, |controller, cx| {
                                        controller.open_local_vault_disable_confirm(window, cx);
                                    });
                                },
                            )
                            .into_any_element()
                        } else {
                            editor_button_with_id(
                                "local-vault-disable-disabled",
                                i18n::string("settings.sync.vault.actions.disable"),
                                false,
                                true,
                                true,
                                |_, _| {},
                            )
                            .into_any_element()
                        }
                    }
                }),
            )
            .description(i18n::string("settings.sync.vault.disable.description")),
        ])
}

fn sync_provider_group(settings: Entity<SettingsController>) -> SettingGroup {
    SettingGroup::new()
        .title(i18n::string("settings.sync.provider_group.title"))
        .description(i18n::string("settings.sync.provider_group.description"))
        .item(
            SettingItem::new(
                i18n::string("settings.sync.provider.backend.label"),
                SettingField::element(SyncProviderField::new(settings)),
            )
            .description(i18n::string("settings.sync.provider.backend.description")),
        )
}

fn render_text_action_field(text: String, action: Option<Button>) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    let row = h_flex().w_full().gap_3().items_center().child(
        div()
            .flex_1()
            .min_w(px(0.0))
            .text_size(miaominal_settings::FontSize::Input.scaled())
            .text_color(rgb(roles.on_surface_variant))
            .child(text),
    );

    match action {
        Some(action) => row.child(action).into_any_element(),
        None => row.into_any_element(),
    }
}

fn render_supporting_action_field(action: Button) -> AnyElement {
    v_flex()
        .w_full()
        .gap_3()
        .child(h_flex().w_full().justify_end().child(action))
        .into_any_element()
}

fn render_text_actions_field(text: String, actions: Vec<AnyElement>) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    v_flex()
        .w_full()
        .gap_3()
        .child(
            div()
                .min_w(px(0.0))
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .text_color(rgb(roles.on_surface_variant))
                .child(text),
        )
        .child(h_flex().w_full().justify_end().gap_3().children(actions))
        .into_any_element()
}

fn sync_encryption_group(settings: Entity<SettingsController>) -> SettingGroup {
    SettingGroup::new()
        .title(i18n::string("settings.sync.encryption_group.title"))
        .description(i18n::string("settings.sync.encryption_group.description"))
        .item(
            SettingItem::new(
                i18n::string("settings.sync.encryption.passphrase.label"),
                SettingField::render({
                    let settings = settings.clone();
                    move |_, _, cx| {
                        let configured = settings.read(cx).passphrase_is_set();
                        let operation_in_progress =
                            settings.read(cx).sync_passphrase_operation_in_progress();
                        let status_text = i18n::string(if configured {
                            "settings.sync.encryption.passphrase.configured_hint"
                        } else {
                            "settings.sync.encryption.passphrase.unconfigured_hint"
                        });

                        if operation_in_progress {
                            return render_text_actions_field(
                                status_text,
                                vec![
                                    div()
                                        .id("settings-sync-passphrase-spinner")
                                        .min_w(px(116.0))
                                        .min_h(px(32.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(md3_spinner(18.0))
                                        .into_any_element(),
                                ],
                            );
                        }

                        let action_label = settings.read(cx).sync_passphrase_action_label();
                        let clear_label = settings.read(cx).sync_passphrase_clear_action_label();
                        let settings_open = settings.clone();
                        let mut actions = vec![
                            settings_action_button(
                                "sync-passphrase-open",
                                action_label,
                                !configured,
                                move |window, cx| {
                                    settings_open.update(cx, |controller, cx| {
                                        controller.open_sync_passphrase_popup(window, cx);
                                    });
                                },
                            )
                            .into_any_element(),
                        ];

                        if configured {
                            let settings_clear = settings.clone();
                            actions.push(
                                settings_action_button(
                                    "sync-passphrase-clear",
                                    clear_label,
                                    false,
                                    move |_window, cx| {
                                        settings_clear.update(cx, |controller, cx| {
                                            controller.open_sync_passphrase_clear_confirm_popup(cx);
                                        });
                                    },
                                )
                                .into_any_element(),
                            );
                        }

                        render_text_actions_field(status_text, actions)
                    }
                }),
            )
            .layout(Axis::Vertical)
            .description(i18n::string(
                "settings.sync.encryption.passphrase.description",
            )),
        )
}

fn sync_status_group(settings: Entity<SettingsController>) -> SettingGroup {
    SettingGroup::new()
        .title(i18n::string("settings.sync.status_group.title"))
        .description(i18n::string("settings.sync.status_group.description"))
        .items(vec![
            SettingItem::new(
                i18n::string("settings.sync.status.last_synced.label"),
                SettingField::render({
                    let settings = settings.clone();
                    move |_, _, cx| {
                        let last_sync_at = settings.read(cx).sync_config().last_sync_at;
                        let text = if last_sync_at == 0 {
                            i18n::string("settings.sync.status.last_synced.never")
                        } else {
                            use std::time::{Duration, UNIX_EPOCH};
                            let system_time = UNIX_EPOCH + Duration::from_secs(last_sync_at);
                            format_local_timestamp(Some(system_time)).to_string()
                        };
                        let roles = miaominal_settings::current_theme().material.roles;
                        div()
                            .text_size(miaominal_settings::FontSize::Input.scaled())
                            .text_color(rgb(roles.on_surface_variant))
                            .child(text)
                            .into_any_element()
                    }
                }),
            )
            .description(i18n::string("settings.sync.status.last_synced.description")),
            SettingItem::new(
                i18n::string("settings.sync.status.state.label"),
                SettingField::render({
                    let settings = settings.clone();
                    move |_, _, cx| {
                        let sync_status = settings.read(cx).sync_status().clone();
                        let status_text =
                            super::super::super::support::sync_status_summary(&sync_status);
                        let roles = miaominal_settings::current_theme().material.roles;
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(miaominal_settings::FontSize::Input.scaled())
                            .text_color(rgb(roles.on_surface_variant))
                            .child(status_text)
                            .into_any_element()
                    }
                }),
            )
            .description(i18n::string("settings.sync.status.state.description")),
            SettingItem::new(
                i18n::string("settings.sync.status.action.label"),
                SettingField::render(move |_options, _, cx| {
                    let settings = settings.clone();
                    let disabled = matches!(settings.read(cx).sync_status(), SyncStatus::Syncing);

                    editor_button_with_id(
                        "sync-now-button",
                        i18n::string("settings.sync.status.action.label"),
                        true,
                        true,
                        disabled,
                        move |window, cx| {
                            settings.update(cx, |controller, cx| {
                                controller.trigger_sync_now(window, cx);
                            });
                        },
                    )
                    .into_any_element()
                }),
            )
            .description(i18n::string("settings.sync.status.action.description")),
        ])
}
