use super::super::*;
use super::session_agent_mentions;
use crate::ui::components::icon_button_with_tooltip;
use crate::ui::i18n;
use crate::ui::shell::actions::ai_provider_kind_chat_supported;
use crate::ui::shell::session_agent::agent_provider_kind;
use gpui::{Animation, AnimationExt as _, ClipboardEntry};
use gpui_component::{Disableable as _, menu::DropdownMenu as _};
use miaominal_agent::{AgentMode, AgentReasoningSupport, agent_reasoning_support};
use miaominal_settings::{AiProviderConfig, AiReasoningEffort};
use std::time::Duration;

const SESSION_AGENT_SEND_PULSE_DURATION: Duration = Duration::from_millis(1100);

fn ai_provider_kind_supports_vision(kind: miaominal_settings::AiProviderKind) -> bool {
    matches!(
        kind,
        miaominal_settings::AiProviderKind::OpenAi
            | miaominal_settings::AiProviderKind::Anthropic
            | miaominal_settings::AiProviderKind::Gemini
            | miaominal_settings::AiProviderKind::OpenRouter
            | miaominal_settings::AiProviderKind::Xai
    )
}

fn selected_ai_provider(settings: &SettingsController, _cx: &App) -> Option<AiProviderConfig> {
    let app_settings = settings.settings();
    app_settings
        .selected_ai_provider_id
        .as_deref()
        .and_then(|selected_id| {
            app_settings
                .ai_providers
                .iter()
                .find(|provider| provider.id == selected_id && provider.enabled)
        })
        .or_else(|| {
            app_settings
                .ai_providers
                .iter()
                .find(|provider| provider.enabled && ai_provider_kind_chat_supported(provider.kind))
        })
        .filter(|provider| ai_provider_kind_chat_supported(provider.kind))
        .cloned()
}

fn reasoning_effort_label_key(effort: AiReasoningEffort) -> &'static str {
    match effort {
        AiReasoningEffort::Default => "workspace.panel.agent.reasoning.default",
        AiReasoningEffort::Low => "workspace.panel.agent.reasoning.low",
        AiReasoningEffort::Medium => "workspace.panel.agent.reasoning.medium",
        AiReasoningEffort::High => "workspace.panel.agent.reasoning.high",
    }
}

fn reasoning_effort_selectable(support: AgentReasoningSupport, effort: AiReasoningEffort) -> bool {
    effort == AiReasoningEffort::Default || support != AgentReasoningSupport::Unsupported
}

#[cfg(test)]
mod reasoning_menu_tests {
    use super::*;

    #[test]
    fn unsupported_models_can_only_select_provider_default() {
        assert!(reasoning_effort_selectable(
            AgentReasoningSupport::Unsupported,
            AiReasoningEffort::Default
        ));
        for effort in [
            AiReasoningEffort::Low,
            AiReasoningEffort::Medium,
            AiReasoningEffort::High,
        ] {
            assert!(!reasoning_effort_selectable(
                AgentReasoningSupport::Unsupported,
                effort
            ));
        }
    }

    #[test]
    fn unknown_models_keep_all_efforts_selectable() {
        for effort in AiReasoningEffort::all() {
            assert!(reasoning_effort_selectable(
                AgentReasoningSupport::Unknown,
                *effort
            ));
        }
    }
}

fn agent_mode_label_key(mode: AgentMode) -> &'static str {
    match mode {
        AgentMode::Ask => "agent.mode.ask",
        AgentMode::Execute => "agent.mode.execute",
        AgentMode::NonBlocking => "agent.mode.non_blocking",
        AgentMode::FullAuto => "agent.mode.full_auto",
    }
}

fn render_session_agent_mode_menu(
    agent: Entity<AgentController>,
    selected_mode: AgentMode,
) -> gpui::AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let mode_label = i18n::string(agent_mode_label_key(selected_mode));
    let menu_agent = agent.clone();

    Button::new("session-agent-mode-menu")
        .ghost()
        .small()
        .compact()
        .dropdown_caret(true)
        .label(mode_label.clone())
        .tooltip(mode_label)
        .w_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .rounded(px(14.0))
        .bg(rgb(roles.surface_container_high))
        .text_color(rgb(roles.on_surface))
        .dropdown_menu(move |menu, _, _| {
            let mut menu = menu.min_w(180.0);
            for mode in [
                AgentMode::Ask,
                AgentMode::Execute,
                AgentMode::NonBlocking,
                AgentMode::FullAuto,
            ] {
                let mode_agent = menu_agent.clone();
                menu = menu.item(
                    PopupMenuItem::new(i18n::string(agent_mode_label_key(mode)))
                        .checked(selected_mode == mode)
                        .on_click(move |_, _, cx| {
                            mode_agent.update(cx, |controller, cx| {
                                controller.select_agent_mode(mode, cx);
                            });
                        }),
                );
            }
            menu
        })
        .into_any_element()
}

fn render_session_agent_provider_menu(
    settings: Entity<SettingsController>,
    cx: &App,
) -> gpui::AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let providers = {
        let controller = settings.read(cx);
        controller
            .settings()
            .ai_providers
            .iter()
            .filter(|provider| provider.enabled && ai_provider_kind_chat_supported(provider.kind))
            .cloned()
            .collect::<Vec<_>>()
    };
    let selected_provider = selected_ai_provider(settings.read(cx), cx);
    let selected_provider_id = selected_provider
        .as_ref()
        .map(|provider| provider.id.clone());
    let selected_effort = selected_provider
        .as_ref()
        .map(|provider| provider.reasoning_effort)
        .unwrap_or_default();
    let reasoning_support =
        selected_provider
            .as_ref()
            .map_or(AgentReasoningSupport::Unknown, |provider| {
                agent_reasoning_support(agent_provider_kind(provider.kind), provider.model.as_str())
            });
    let effort_label = i18n::string(reasoning_effort_label_key(selected_effort));
    let provider_label = selected_provider
        .as_ref()
        .map(|provider| provider.name.clone())
        .unwrap_or_else(|| i18n::string("workspace.panel.agent.provider_menu.empty"));
    let tooltip = selected_provider.as_ref().map_or_else(
        || i18n::string("workspace.panel.agent.no_provider_configured"),
        |provider| match reasoning_support {
            AgentReasoningSupport::Supported => i18n::string_args(
                "workspace.panel.agent.tooltips.provider_and_reasoning",
                &[
                    ("provider", provider.name.as_str()),
                    ("level", effort_label.as_str()),
                ],
            ),
            AgentReasoningSupport::Unsupported => i18n::string_args(
                "workspace.panel.agent.tooltips.provider_reasoning_unsupported",
                &[
                    ("provider", provider.name.as_str()),
                    ("model", provider.model.as_str()),
                    ("level", effort_label.as_str()),
                ],
            ),
            AgentReasoningSupport::Unknown => i18n::string_args(
                "workspace.panel.agent.tooltips.provider_reasoning_unknown",
                &[
                    ("provider", provider.name.as_str()),
                    ("model", provider.model.as_str()),
                    ("level", effort_label.as_str()),
                ],
            ),
        },
    );
    let has_provider = selected_provider_id.is_some();
    let menu_settings = settings.clone();
    let menu_providers = providers.clone();
    let menu_selected_provider_id = selected_provider_id.clone();
    let reasoning_submenu_label = if reasoning_support == AgentReasoningSupport::Unsupported {
        i18n::string_args(
            "workspace.panel.agent.provider_menu.reasoning_unsupported",
            &[("level", effort_label.as_str())],
        )
    } else {
        i18n::string_args(
            "workspace.panel.agent.provider_menu.reasoning",
            &[("level", effort_label.as_str())],
        )
    };

    Button::new("session-agent-provider-menu")
        .ghost()
        .small()
        .compact()
        .dropdown_caret(true)
        .label(provider_label)
        .tooltip(tooltip)
        .disabled(!has_provider)
        .w_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .rounded(px(14.0))
        .bg(rgb(roles.surface_container_high))
        .text_color(rgb(roles.on_surface))
        .dropdown_menu(move |menu, window, cx| {
            let mut menu = menu.min_w(180.0);
            for provider in &menu_providers {
                let provider_id = provider.id.clone();
                let provider_settings = menu_settings.clone();
                menu = menu.item(
                    PopupMenuItem::new(provider.name.clone())
                        .checked(menu_selected_provider_id.as_deref() == Some(provider.id.as_str()))
                        .on_click(move |_, window, cx| {
                            let provider_id = provider_id.clone();
                            provider_settings.update(cx, |controller, cx| {
                                controller.select_ai_provider(provider_id, window, cx);
                            });
                        }),
                );
            }

            let Some(provider_id) = menu_selected_provider_id.clone() else {
                return menu;
            };
            let reasoning_settings = menu_settings.clone();
            menu.separator().submenu(
                reasoning_submenu_label.clone(),
                window,
                cx,
                move |menu, _, _| {
                    let mut menu = menu;
                    for effort in AiReasoningEffort::all() {
                        let effort = *effort;
                        let effort_settings = reasoning_settings.clone();
                        let effort_provider_id = provider_id.clone();
                        menu = menu.item(
                            PopupMenuItem::new(i18n::string(reasoning_effort_label_key(effort)))
                                .checked(selected_effort == effort)
                                .disabled(!reasoning_effort_selectable(reasoning_support, effort))
                                .on_click(move |_, _, cx| {
                                    effort_settings.update(cx, |controller, cx| {
                                        controller.set_ai_provider_reasoning_effort(
                                            &effort_provider_id,
                                            effort,
                                            cx,
                                        );
                                    });
                                }),
                        );
                    }
                    menu
                },
            )
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_session_agent_composer(
    agent_controller: &AgentController,
    agent: Entity<AgentController>,
    settings: Entity<SettingsController>,
    cx: &App,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    let state = agent_controller.session_agent();
    let prompt_input = agent_controller.prompt_input();
    let selected_agent_mode = agent_controller.agent_mode();
    let prompt_menu_input = prompt_input.clone();
    let pty_toggle_controller = agent.clone();
    let attach_controller = agent.clone();
    let paste_controller = agent.clone();
    let badge_controller = agent.clone();
    let mention_controller = agent.clone();
    let send_controller = agent.clone();
    let waiting = state.is_busy();
    let has_attachments = !state.pending_attachments.is_empty();
    let has_pending_images = state
        .pending_attachments
        .iter()
        .any(|attachment| attachment.is_image());
    let has_targets = !state.selected_at_targets.is_empty();
    let at_mention_query = state.at_mention_query.clone();
    let selected_at_targets = state.selected_at_targets.clone();
    let pending_attachments = state.pending_attachments.clone();
    let exec_mode_is_pty = state.exec_mode.is_pty();
    let target_candidates = agent_controller.target_candidates();
    let selected_provider_kind =
        selected_ai_provider(settings.read(cx), cx).map(|provider| provider.kind);
    let has_provider = selected_provider_kind.is_some();
    let image_text_fallback = has_pending_images
        && selected_provider_kind.is_some_and(|kind| !ai_provider_kind_supports_vision(kind));

    div()
        .flex_shrink_0()
        .p_2()
        .relative()
        .on_drop::<ExternalPaths>({
            let drop_controller = agent.clone();
            move |paths: &ExternalPaths, _window, cx| {
                let local_paths: Vec<std::path::PathBuf> = paths.paths().to_vec();
                drop_controller.update(cx, |controller, cx| {
                    controller.ingest_attachment_paths_and_report(local_paths, cx);
                });
            }
        })
        .child(
            v_flex()
                .w_full()
                .gap_2()
                .when_some(at_mention_query, |this, query| {
                    this.child(
                        session_agent_mentions::render_session_agent_at_mention_popup(
                            mention_controller.clone(),
                            target_candidates.clone(),
                            query,
                        ),
                    )
                })
                .child(
                    v_flex()
                        .rounded(px(8.0))
                        .bg(rgb(roles.surface_container_high))
                        .p_2()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_h(px(86.0))
                                .max_h(px(190.0))
                                .rounded(px(6.0))
                                .relative()
                                .overflow_hidden()
                                .id("session-agent-prompt-input-menu")
                                .when(has_targets || has_attachments, |this| {
                                    this.child(div().flex_shrink_0().child(
                                        render_composer_badge_row(
                                            badge_controller.clone(),
                                            roles,
                                            has_targets,
                                            has_attachments,
                                            target_candidates.clone(),
                                            selected_at_targets.clone(),
                                            pending_attachments.clone(),
                                        ),
                                    ))
                                })
                                .when(image_text_fallback, |this| {
                                    this.child(
                                        h_flex()
                                            .w_full()
                                            .gap_1()
                                            .items_center()
                                            .px_1()
                                            .py_1()
                                            .text_size(miaominal_settings::FontSize::Body.scaled())
                                            .text_color(rgb(text_muted))
                                            .child(Icon::new(AppIcon::Sparkles).small())
                                            .child(i18n::string(
                                                "workspace.panel.agent.messages.image_attachments_text_fallback",
                                            )),
                                    )
                                })
                                .child(
                                    div().flex_1().child(
                                        HintedInput::new(&prompt_input)
                                            .w_full()
                                            .appearance(false)
                                            .focus_bordered(false)
                                            .p_1()
                                            .hint_left(px(4.0))
                                            .hint_top(px(4.0))
                                            .hint_bottom(px(4.0)),
                                    ),
                                )
                                .on_key_down({
                                    let controller = paste_controller.clone();
                                    move |event: &KeyDownEvent, _window, cx| {
                                        handle_paste_key(event, controller.clone(), cx);
                                    }
                                })
                                .context_menu(move |menu, _window, cx| {
                                    let state = prompt_menu_input.read(cx);
                                    let has_selection = !state.selected_range().is_empty();
                                    let has_text = !state.value().is_empty();
                                    let focus = state.focus_handle(cx);
                                    menu.action_context(focus)
                                        .menu_with_disabled(
                                            i18n::string("workspace.menu.cut"),
                                            Box::new(gpui_component::input::Cut),
                                            !has_selection,
                                        )
                                        .menu_with_disabled(
                                            i18n::string("workspace.menu.copy"),
                                            Box::new(gpui_component::input::Copy),
                                            !has_selection,
                                        )
                                        .menu_with_disabled(
                                            i18n::string("workspace.menu.paste"),
                                            Box::new(gpui_component::input::Paste),
                                            cx.read_from_clipboard().is_none(),
                                        )
                                        .item(PopupMenuItem::separator())
                                        .menu_with_disabled(
                                            i18n::string("workspace.menu.select_all"),
                                            Box::new(gpui_component::input::SelectAll),
                                            !has_text,
                                        )
                                }),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .h(px(32.0))
                                .items_center()
                                .gap_2()
                                .child(
                                    div().w(px(112.0)).min_w(px(0.0)).child(
                                        render_session_agent_provider_menu(settings.clone(), cx),
                                    ),
                                )
                                .child(
                                    div().w(px(112.0)).min_w(px(0.0)).child(
                                        render_session_agent_mode_menu(
                                            agent.clone(),
                                            selected_agent_mode,
                                        ),
                                    ),
                                )
                                .child(icon_button_with_tooltip(
                                    AppIcon::Paperclip,
                                    i18n::string("workspace.panel.agent.tooltips.attach_file"),
                                    24.0,
                                    8.0,
                                    Some(roles.surface_container_high),
                                    Some(text_muted),
                                    None,
                                    move |_window, cx| {
                                        let controller = attach_controller.clone();
                                        controller.update(cx, |controller, cx| {
                                            controller.open_attachment_picker(cx);
                                        });
                                    },
                                ))
                                .child(icon_button_with_tooltip(
                                    AppIcon::LaptopMinimal,
                                    i18n::string(if exec_mode_is_pty {
                                        "workspace.panel.agent.tooltips.disable_pty"
                                    } else {
                                        "workspace.panel.agent.tooltips.enable_pty"
                                    }),
                                    24.0,
                                    8.0,
                                    Some(if exec_mode_is_pty {
                                        roles.secondary_container
                                    } else {
                                        roles.surface_container_high
                                    }),
                                    Some(if exec_mode_is_pty {
                                        roles.on_secondary_container
                                    } else {
                                        text_muted
                                    }),
                                    None,
                                    move |_window, cx| {
                                        let controller = pty_toggle_controller.clone();
                                        controller.update(cx, |controller, cx| {
                                            controller.toggle_execution_mode(cx);
                                        });
                                    },
                                ))
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .id("session-agent-send-action")
                                        .child(icon_button_with_tooltip(
                                            if waiting {
                                                AppIcon::Pause
                                            } else {
                                                AppIcon::ChevronUp
                                            },
                                            if !has_provider && !waiting {
                                                i18n::string(
                                                    "workspace.panel.agent.no_provider_configured",
                                                )
                                            } else {
                                                i18n::string(if waiting {
                                                "workspace.panel.agent.tooltips.stop_response"
                                            } else {
                                                "workspace.panel.agent.tooltips.send_message"
                                                })
                                            },
                                            26.0,
                                            8.0,
                                            Some(if waiting {
                                                roles.error_container
                                            } else if !has_provider {
                                                roles.surface_container_highest
                                            } else {
                                                roles.primary
                                            }),
                                            Some(if waiting {
                                                roles.on_error_container
                                            } else if !has_provider {
                                                text_muted
                                            } else {
                                                roles.on_primary
                                            }),
                                            None,
                                            move |window, cx| {
                                                let controller = send_controller.clone();
                                                controller.update(cx, |controller, cx| {
                                                    if controller.session_agent().is_busy() {
                                                        controller.stop_session_agent_stream(cx);
                                                    } else {
                                                        controller
                                                            .submit_session_agent_prompt(window, cx);
                                                    }
                                                });
                                            },
                                        ))
                                        .with_animation(
                                            SharedString::from(format!(
                                                "session-agent-send-state-{waiting}"
                                            )),
                                            if waiting {
                                                Animation::new(SESSION_AGENT_SEND_PULSE_DURATION)
                                                    .repeat()
                                                    .with_easing(gpui::bounce(gpui::ease_in_out))
                                            } else {
                                                short_feedback_animation()
                                            },
                                            move |element, delta| {
                                                if waiting {
                                                    element.opacity(0.72 + delta * 0.28)
                                                } else {
                                                    element.opacity(0.64 + delta * 0.36)
                                                }
                                            },
                                        ),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

/// Handles Ctrl+V / Cmd+V in the composer: if the clipboard holds an image,
/// ingest it as an attachment; otherwise let the default text paste proceed.
fn handle_paste_key(event: &KeyDownEvent, controller: Entity<AgentController>, cx: &mut gpui::App) {
    let keystroke = &event.keystroke;
    let is_paste = (keystroke.key == "v" || keystroke.key == "V")
        && (keystroke.modifiers.control || keystroke.modifiers.platform);
    if !is_paste {
        return;
    }
    let Some(item) = cx.read_from_clipboard() else {
        return;
    };
    for entry in item.entries() {
        if let ClipboardEntry::Image(image) = entry {
            let bytes = image.bytes.clone();
            let format = image.format;
            controller.update(cx, |controller, cx| {
                controller.ingest_clipboard_image_and_report(format, bytes, cx);
            });
            return;
        }
    }
}

/// Renders target-chips and attachment badges in a single flex-wrap row
/// using the same pill style (`.rounded(px(999.0))`). Attachment badges
/// show a filename with a Close button; target chips show `@name` with
/// a Close button.
fn render_composer_badge_row(
    agent: Entity<AgentController>,
    roles: miaominal_settings::theme::Md3Roles,
    has_targets: bool,
    has_attachments: bool,
    candidates: Vec<SessionAgentTargetCandidate>,
    selected_at_targets: Vec<String>,
    pending_attachments: Vec<miaominal_core::chat_attachment::ChatAttachment>,
) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .gap_1()
        .flex_wrap()
        .when(has_targets, |this| {
            let names = selected_at_targets.clone();
            this.children(names.into_iter().map(|name| {
                let remove_name = name.clone();
                let badge_id =
                    SharedString::from(format!("session-agent-target-badge-{}", name.as_str()));
                let remove_id =
                    SharedString::from(format!("session-agent-target-remove-{}", name.as_str()));
                let remove_controller = agent.clone();
                let resolved = candidates.iter().any(|candidate| {
                    candidate.name == name
                        || candidate
                            .name
                            .strip_prefix(&name)
                            .is_some_and(|suffix| suffix.starts_with(' '))
                });
                div()
                    .id(badge_id)
                    .flex_none()
                    .px_2()
                    .py_1()
                    .rounded(px(999.0))
                    .bg(rgb(if resolved {
                        roles.secondary_container
                    } else {
                        roles.error_container
                    }))
                    .text_color(rgb(if resolved {
                        roles.on_secondary_container
                    } else {
                        roles.on_error_container
                    }))
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(format!("@{name}"))
                            .child(
                                div()
                                    .id(remove_id)
                                    .size(px(16.0))
                                    .rounded(px(4.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        move |_, _window, cx| {
                                            let controller = remove_controller.clone();
                                            let name = remove_name.clone();
                                            controller.update(cx, |controller, cx| {
                                                controller.remove_at_target(&name, cx);
                                            });
                                        },
                                    )
                                    .child(Icon::new(AppIcon::Close).size(px(12.0)).text_color(
                                        rgb(if resolved {
                                            roles.on_secondary_container
                                        } else {
                                            roles.on_error_container
                                        }),
                                    )),
                            ),
                    )
                    .into_any_element()
            }))
        })
        .when(has_attachments, |this| {
            let attachments = pending_attachments.clone();
            this.children(attachments.iter().map(|attachment| {
                let attachment_id = attachment.id.clone();
                let filename = SharedString::from(attachment.filename.clone());
                let remove_controller = agent.clone();
                let remove_id = attachment_id.clone();
                let icon = match &attachment.content {
                    miaominal_core::chat_attachment::ChatAttachmentContent::Image(_) => {
                        AppIcon::Upload
                    }
                    miaominal_core::chat_attachment::ChatAttachmentContent::TextFile(_) => {
                        AppIcon::File
                    }
                };
                let bg = roles.secondary_container;
                let fg = roles.on_secondary_container;
                div()
                    .id(SharedString::from(format!(
                        "attachment-badge-{}",
                        attachment_id.as_str()
                    )))
                    .flex_none()
                    .px_2()
                    .py_1()
                    .rounded(px(999.0))
                    .bg(rgb(bg))
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(Icon::new(icon).small().text_color(rgb(fg)))
                            .child(
                                div()
                                    .min_w(px(0.0))
                                    .text_color(rgb(fg))
                                    .child(truncate_with_ellipsis(filename.as_ref(), 24)),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "attachment-badge-remove-{attachment_id}"
                                    )))
                                    .size(px(16.0))
                                    .rounded(px(4.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        move |_, _window, cx| {
                                            let controller = remove_controller.clone();
                                            let id = remove_id.clone();
                                            controller.update(cx, |controller, cx| {
                                                controller.remove_pending_attachment_and_report(
                                                    id.as_ref(),
                                                    cx,
                                                );
                                            });
                                        },
                                    )
                                    .child(
                                        Icon::new(AppIcon::Close)
                                            .size(px(12.0))
                                            .text_color(rgb(fg)),
                                    ),
                            ),
                    )
                    .into_any_element()
            }))
        })
        .into_any_element()
}
