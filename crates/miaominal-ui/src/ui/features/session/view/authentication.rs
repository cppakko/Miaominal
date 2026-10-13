//! Authentication surfaces capture their request identity and input entities.
//! Retained views remain renderable while stale actions cannot touch a new request.
use crate::ui::components::animation::render_basic_dialog;
use crate::ui::components::overlay_target::OverlayTarget;
use crate::ui::components::{BasicDialogActionTone, HintedInput, basic_dialog_action_button};
use crate::ui::contracts::overlay::OverlaySnapshot;
use crate::ui::contracts::workspace::TabId;
use crate::ui::features::session::SessionController;
use crate::ui::i18n;
use gpui_kit::component::{Sizable as _, h_flex, input::InputState, v_flex};
use gpui_kit::{App, Entity, Focusable as _, SharedString, div, prelude::*, px, rgb};
use miaominal_ssh::{HostKeyDecision, HostKeyPrompt, KbiChallenge};

pub(in crate::ui::features::session) fn host_key_overlay_snapshot(
    entity: Entity<SessionController>,
    tab_id: TabId,
    request_id: u64,
    prompt: HostKeyPrompt,
) -> OverlaySnapshot {
    let target = OverlayTarget::new(entity, move |owner| {
        owner.is_host_key_request_current(tab_id, request_id)
    });
    let dismiss = target.clone();
    OverlaySnapshot::new(
        format!("session-host-key-{tab_id}"),
        request_id,
        move |exit, height, _| {
            let decision_target = target.clone();
            crate::ui::components::host_key_prompt::render_host_key_prompt(
                move |decision, cx| {
                    decision_target.update(cx, |owner, cx| {
                        owner.resolve_host_key_prompt(tab_id, decision, cx);
                    });
                },
                &prompt,
                exit,
                height,
                format!("session-host-key-{tab_id}"),
            )
        },
        move |_, cx| {
            dismiss.update(cx, |owner, cx| {
                owner.resolve_host_key_prompt(tab_id, HostKeyDecision::Reject, cx);
            });
        },
    )
}

pub(in crate::ui::features::session) fn keyboard_interactive_overlay_snapshot(
    entity: Entity<SessionController>,
    tab_id: TabId,
    request_id: u64,
    challenge: KbiChallenge,
    inputs: Vec<Entity<InputState>>,
    cx: &App,
) -> OverlaySnapshot {
    let target = OverlayTarget::new(entity, move |owner| {
        owner.is_keyboard_interactive_request_current(tab_id, request_id)
    });
    let dismiss = target.clone();
    let submit = target.clone();
    let submit_inputs = inputs.clone();
    let focus = inputs.first().map(|input| input.read(cx).focus_handle(cx));
    let snapshot = OverlaySnapshot::new(
        format!("session-keyboard-interactive-{tab_id}"),
        request_id,
        move |exit, _, _| {
            render_keyboard_interactive_prompt(target.clone(), tab_id, &challenge, &inputs, exit)
        },
        move |_, cx| {
            dismiss.update(cx, |owner, cx| {
                owner.cancel_keyboard_interactive(tab_id, cx)
            });
        },
    )
    .with_submit(move |_, cx| {
        let responses = input_values(&submit_inputs, cx);
        submit.update(cx, |owner, cx| {
            owner.submit_keyboard_interactive(tab_id, responses, cx);
        });
    });
    if let Some(focus) = focus {
        snapshot.with_keyboard_focus(focus)
    } else {
        snapshot
    }
}

fn input_values(inputs: &[Entity<InputState>], cx: &App) -> Vec<String> {
    inputs
        .iter()
        .map(|input| input.read(cx).value().to_string())
        .collect()
}

fn render_keyboard_interactive_prompt(
    target: OverlayTarget<SessionController>,
    tab_id: TabId,
    challenge: &KbiChallenge,
    inputs: &[Entity<InputState>],
    exit_progress: Option<f32>,
) -> gpui_kit::AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let title = if challenge.name.is_empty() {
        i18n::string("prompts.authentication_challenge")
    } else {
        challenge.name.clone()
    };
    let submit_target = target.clone();
    let submit_inputs = inputs.to_vec();
    let submit_button = basic_dialog_action_button(
        "keyboard-interactive-submit",
        i18n::string("dialogs.common.submit"),
        BasicDialogActionTone::Default,
    )
    .on_click(move |_, _, cx| {
        let responses = input_values(&submit_inputs, cx);
        submit_target.update(cx, |owner, cx| {
            owner.submit_keyboard_interactive(tab_id, responses, cx);
        });
    });
    let cancel_button = basic_dialog_action_button(
        "keyboard-interactive-cancel",
        i18n::string("dialogs.common.cancel"),
        BasicDialogActionTone::Default,
    )
    .on_click(move |_, _, cx| {
        target.update(cx, |owner, cx| {
            owner.cancel_keyboard_interactive(tab_id, cx)
        });
    });
    let body = v_flex()
        .w_full()
        .gap_4()
        .children(challenge.prompts.iter().zip(inputs).map(|(prompt, input)| {
            v_flex()
                .id(SharedString::from(format!(
                    "keyboard-interactive-prompt-{:?}",
                    input.entity_id()
                )))
                .w_full()
                .gap_1()
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(roles.on_surface_variant))
                        .child(prompt.prompt.clone()),
                )
                .child(
                    HintedInput::new(input)
                        .large()
                        .w_full()
                        .border_0()
                        .rounded(px(14.0))
                        .bg(rgb(roles.surface_container_low)),
                )
                .into_any_element()
        }))
        .into_any_element();
    render_basic_dialog(
        "keyboard-interactive",
        title,
        (!challenge.instructions.is_empty()).then(|| challenge.instructions.clone()),
        Some(body),
        h_flex()
            .gap_2()
            .justify_end()
            .child(cancel_button)
            .child(submit_button)
            .into_any_element(),
        exit_progress,
    )
}
