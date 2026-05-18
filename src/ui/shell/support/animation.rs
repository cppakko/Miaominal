#![allow(dead_code)]

use super::super::*;
use gpui::{Animation, AnimationExt as _, ease_in_out, linear};

pub(in crate::ui::shell) const SHORT_FEEDBACK_DURATION: Duration = Duration::from_millis(120);
pub(in crate::ui::shell) const CONTAINER_TRANSITION_DURATION: Duration = Duration::from_millis(180);
pub(in crate::ui::shell) const OVERLAY_ENTER_DURATION: Duration = Duration::from_millis(220);
pub(in crate::ui::shell) const LIST_ENTER_DURATION: Duration = Duration::from_millis(160);

const PAGE_CONTAINER_SCENE: &str = "page-container";
const OVERLAY_SCENE: &str = "overlay";
const BOTTOM_POPUP_SCENE: &str = "bottom-popup";
const EMPTY_STATE_SCENE: &str = "empty-state";
const STATUS_INDICATOR_SCENE: &str = "status-indicator";
const PAGE_CONTAINER_ENTER_OFFSET: f32 = 10.0;
const BOTTOM_POPUP_ENTER_OFFSET: f32 = 32.0;

pub(in crate::ui::shell) fn short_feedback_animation() -> Animation {
    Animation::new(SHORT_FEEDBACK_DURATION).with_easing(linear)
}

pub(in crate::ui::shell) fn container_transition_animation() -> Animation {
    Animation::new(CONTAINER_TRANSITION_DURATION).with_easing(ease_in_out)
}

pub(in crate::ui::shell) fn overlay_enter_animation() -> Animation {
    Animation::new(OVERLAY_ENTER_DURATION).with_easing(ease_in_out)
}

pub(in crate::ui::shell) fn list_enter_animation() -> Animation {
    Animation::new(LIST_ENTER_DURATION).with_easing(ease_in_out)
}

pub(in crate::ui::shell) fn shell_animation_id(
    scene: &'static str,
    stable_key: impl AsRef<str>,
) -> SharedString {
    SharedString::from(format!("{scene}-{}", stable_key.as_ref()))
}

pub(in crate::ui::shell) fn page_container_animation_id(section: SidebarSection) -> SharedString {
    shell_animation_id(PAGE_CONTAINER_SCENE, sidebar_section_animation_key(section))
}

pub(in crate::ui::shell) fn overlay_animation_id(stable_key: impl AsRef<str>) -> SharedString {
    shell_animation_id(OVERLAY_SCENE, stable_key)
}

pub(in crate::ui::shell) fn bottom_popup_animation_id(stable_key: impl AsRef<str>) -> SharedString {
    shell_animation_id(BOTTOM_POPUP_SCENE, stable_key)
}

pub(in crate::ui::shell) fn empty_state_animation_id(stable_key: impl AsRef<str>) -> SharedString {
    shell_animation_id(EMPTY_STATE_SCENE, stable_key)
}

pub(in crate::ui::shell) fn status_indicator_animation_id(
    stable_key: impl AsRef<str>,
) -> SharedString {
    shell_animation_id(STATUS_INDICATOR_SCENE, stable_key)
}

pub(in crate::ui::shell) fn render_sidebar_page_container(
    panel: AnyElement,
    section: SidebarSection,
    animate: bool,
) -> AnyElement {
    let container = div()
        .relative()
        .flex_1()
        .w_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .child(panel);

    if !animate {
        return container.into_any_element();
    }

    container
        .with_animation(
            page_container_animation_id(section),
            container_transition_animation(),
            |element, delta| {
                element
                    .opacity(delta)
                    .top(px((1.0 - delta) * PAGE_CONTAINER_ENTER_OFFSET))
            },
        )
        .into_any_element()
}

pub(in crate::ui::shell) fn render_prompt_overlay(
    panel: AnyElement,
    stable_key: impl AsRef<str>,
    exit_progress: Option<f32>,
) -> AnyElement {
    let stable_key = stable_key.as_ref().to_string();
    let scrim = settings::current_theme().material.roles.scrim;
    let overlay = div()
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        });

    if let Some(exit_progress) = exit_progress {
        let delta = (1.0 - exit_progress).clamp(0.0, 1.0);
        return overlay
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(color_with_alpha(scrim, (136.0 * delta).round() as u8)),
            )
            .child(div().relative().w_full().opacity(delta).child(panel))
            .into_any_element();
    }

    overlay
        .child(div().absolute().inset_0().with_animation(
            overlay_animation_id(format!("{stable_key}-backdrop")),
            overlay_enter_animation(),
            move |element, delta| {
                element.bg(color_with_alpha(scrim, (136.0 * delta).round() as u8))
            },
        ))
        .child(div().relative().w_full().child(panel).with_animation(
            overlay_animation_id(format!("{stable_key}-panel")),
            overlay_enter_animation(),
            |element, delta| element.opacity(delta),
        ))
        .into_any_element()
}

pub(in crate::ui::shell) fn render_basic_dialog(
    stable_key: impl AsRef<str>,
    title: String,
    supporting_text: Option<String>,
    body: Option<AnyElement>,
    actions: AnyElement,
    exit_progress: Option<f32>,
) -> AnyElement {
    render_basic_dialog_with_config(
        stable_key,
        title,
        supporting_text,
        body,
        actions,
        None,
        BasicDialogHeaderAlignment::Start,
        exit_progress,
    )
}

pub(in crate::ui::shell) fn render_basic_dialog_with_config(
    stable_key: impl AsRef<str>,
    title: String,
    supporting_text: Option<String>,
    body: Option<AnyElement>,
    actions: AnyElement,
    icon: Option<BasicDialogIcon>,
    header_alignment: BasicDialogHeaderAlignment,
    exit_progress: Option<f32>,
) -> AnyElement {
    render_prompt_overlay(
        basic_dialog_panel(
            title,
            supporting_text,
            body,
            actions,
            icon,
            header_alignment,
        ),
        stable_key,
        exit_progress,
    )
}

pub(in crate::ui::shell) fn render_bottom_popup(
    panel: AnyElement,
    stable_key: impl AsRef<str>,
    exit_progress: Option<f32>,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let stable_key = stable_key.as_ref().to_string();
    let scrim = settings::current_theme().material.roles.scrim;
    let overlay = div()
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_end()
        .justify_center()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            on_dismiss(window, cx);
            cx.stop_propagation();
        });

    if let Some(exit_progress) = exit_progress {
        let delta = (1.0 - exit_progress).clamp(0.0, 1.0);
        return overlay
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(color_with_alpha(scrim, (136.0 * delta).round() as u8)),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .top(px((1.0 - delta) * BOTTOM_POPUP_ENTER_OFFSET))
                    .opacity(delta)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(panel),
            )
            .into_any_element();
    }

    overlay
        .child(div().absolute().inset_0().with_animation(
            bottom_popup_animation_id(format!("{stable_key}-backdrop")),
            overlay_enter_animation(),
            move |element, delta| {
                element.bg(color_with_alpha(scrim, (136.0 * delta).round() as u8))
            },
        ))
        .child(
            div()
                .relative()
                .w_full()
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .child(panel)
                .with_animation(
                    bottom_popup_animation_id(format!("{stable_key}-panel")),
                    overlay_enter_animation(),
                    |element, delta| {
                        element
                            .opacity(delta)
                            .top(px((1.0 - delta) * BOTTOM_POPUP_ENTER_OFFSET))
                    },
                ),
        )
        .into_any_element()
}

fn sidebar_section_animation_key(section: SidebarSection) -> &'static str {
    match section {
        SidebarSection::Hosts => "hosts",
        SidebarSection::Keychain => "keychain",
        SidebarSection::PortForwarding => "port-forwarding",
        SidebarSection::Snippets => "snippets",
        SidebarSection::KnownHosts => "known-hosts",
        SidebarSection::Settings => "settings",
    }
}
