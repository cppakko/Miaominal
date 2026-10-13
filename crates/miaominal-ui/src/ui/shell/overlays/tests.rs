//! Exercises the production window overlay compositor with independent owners.

use super::{OverlayScope, WindowOverlayState};
use crate::ui::contracts::navigation::SidebarSection;
use crate::ui::contracts::overlay::OverlaySnapshot;
use crate::ui::contracts::workspace::{PaneId, TabId};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, Bounds, Context, Entity, EventEmitter, Focusable, Subscription,
    TestAppContext, Window, WindowBounds, WindowOptions, div, px, size,
};
use std::collections::HashMap;

#[derive(Clone)]
struct Prompt {
    key: &'static str,
    revision: u64,
    input: Entity<InputState>,
    dismissible: bool,
}

struct Owner {
    prompts: Vec<Prompt>,
    revision: u64,
    dismissals: Vec<&'static str>,
    submissions: HashMap<&'static str, usize>,
    routed_pointer_activations: usize,
}

struct Dismissed(OverlaySnapshot);
impl EventEmitter<Dismissed> for Owner {}

impl Owner {
    fn open(
        &mut self,
        key: &'static str,
        dismissible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.revision += 1;
        self.prompts.retain(|prompt| prompt.key != key);
        self.prompts.push(Prompt {
            key,
            revision: self.revision,
            input: cx.new(|cx| InputState::new(window, cx)),
            dismissible,
        });
        cx.notify();
    }

    fn is_current(&self, key: &str, revision: u64) -> bool {
        self.prompts
            .iter()
            .any(|prompt| prompt.key == key && prompt.revision == revision)
    }

    fn close(&mut self, key: &'static str, revision: u64, cx: &mut Context<Self>) {
        let Some(index) = self
            .prompts
            .iter()
            .position(|prompt| prompt.key == key && prompt.revision == revision)
        else {
            return;
        };
        let snapshot = Self::snapshot(self.prompts.remove(index), cx.entity(), cx);
        self.dismissals.push(key);
        cx.emit(Dismissed(snapshot));
        cx.notify();
    }

    fn submit(&mut self, key: &'static str, revision: u64, cx: &mut Context<Self>) {
        if self.is_current(key, revision) {
            *self.submissions.entry(key).or_default() += 1;
            cx.notify();
        }
    }

    fn snapshot(prompt: Prompt, entity: Entity<Self>, cx: &App) -> OverlaySnapshot {
        let key = prompt.key;
        let revision = prompt.revision;
        let input = prompt.input;
        let focus = input.read(cx).focus_handle(cx);
        let render_owner = entity.clone();
        let dismiss_owner = entity.clone();
        let snapshot = OverlaySnapshot::new(
            key,
            revision,
            move |_, _, _| {
                let submit = render_owner.clone();
                let open = render_owner.clone();
                let pointer_probe = render_owner.clone();
                let form = div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w_64()
                    .when(key != "buttons", |form| {
                        form.child(Input::new(&input).id(format!("{key}-input")))
                    })
                    .child(Button::new(format!("{key}-submit")).label("Save").on_click(
                        move |_, _, cx| {
                            submit.update(cx, |owner, cx| owner.submit(key, revision, cx));
                        },
                    ))
                    .child(
                        Button::new(format!("{key}-pointer-probe"))
                            .label("Pointer probe")
                            .on_click(move |_, _, cx| {
                                pointer_probe.update(cx, |owner, _| {
                                    owner.routed_pointer_activations += 1;
                                });
                            }),
                    )
                    .when(key == "lower", |form| {
                        form.child(
                            Button::new("open-upper")
                                .label("Open nested form…")
                                .on_click(move |_, window, cx| {
                                    open.update(cx, |owner, cx| {
                                        if owner.is_current(key, revision) {
                                            owner.open("upper", true, window, cx);
                                        }
                                    });
                                }),
                        )
                    });
                // Distinct corners let a click target the covered lower form
                // without coinciding with any upper form control.
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .p_8()
                    .when(key == "upper", |surface| surface.items_end().justify_end())
                    // Modal backdrop hit testing belongs to the owner-rendered
                    // surface, as in the production business dialog primitive.
                    .when(key == "buttons", |surface| surface.occlude())
                    .child(form)
                    .into_any_element()
            },
            move |_, cx| {
                dismiss_owner.update(cx, |owner, cx| owner.close(key, revision, cx));
            },
        )
        .escape_dismissible(prompt.dismissible)
        .with_submit(move |_, cx| {
            entity.update(cx, |owner, cx| owner.submit(key, revision, cx));
        });
        if key == "buttons" {
            snapshot
        } else {
            snapshot.with_keyboard_focus(focus)
        }
    }

    fn snapshots(&self, entity: Entity<Self>, cx: &App) -> Vec<OverlaySnapshot> {
        self.prompts
            .iter()
            .cloned()
            .map(|prompt| Self::snapshot(prompt, entity.clone(), cx))
            .collect()
    }
}

struct Harness {
    owner: Entity<Owner>,
    overlays: WindowOverlayState,
    trigger: Entity<InputState>,
    fallback: Entity<InputState>,
    scope: OverlayScope,
    lower_dismissible: bool,
    background_keys: usize,
    _subscriptions: Vec<Subscription>,
}

impl Harness {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let owner = cx.new(|_| Owner {
            prompts: Vec::new(),
            revision: 0,
            dismissals: Vec::new(),
            submissions: HashMap::new(),
            routed_pointer_activations: 0,
        });
        let observer = cx.observe_in(&owner, window, |this, _, window, cx| {
            this.sync(window, cx);
            cx.notify();
        });
        let dismissals =
            cx.subscribe_in(&owner, window, |this, _, event: &Dismissed, window, cx| {
                this.overlays.begin_exit(event.0.clone());
                this.sync(window, cx);
                cx.notify();
            });
        let weak = cx.weak_entity();
        let interceptor = cx.intercept_keystrokes(move |event, window, cx| {
            let modifiers = event.keystroke.modifiers;
            if modifiers.control || modifiers.alt || modifiers.platform || modifiers.shift {
                return;
            }
            let consumed = weak.upgrade().is_some_and(|view| {
                view.update(cx, |this, cx| {
                    this.overlays.remember_trigger(window, cx);
                    let consumed =
                        this.overlays
                            .handle_plain_key(event.keystroke.key.as_str(), window, cx);
                    if consumed {
                        this.sync(window, cx);
                        cx.notify();
                    }
                    consumed
                })
            });
            if consumed {
                cx.stop_propagation();
            }
        });
        Self {
            owner,
            overlays: WindowOverlayState::default(),
            trigger: cx.new(|cx| InputState::new(window, cx)),
            fallback: cx.new(|cx| InputState::new(window, cx)),
            scope: initial_scope(),
            lower_dismissible: true,
            background_keys: 0,
            _subscriptions: vec![observer, dismissals, interceptor],
        }
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshots = self.owner.read(cx).snapshots(self.owner.clone(), cx);
        let fallback = self.fallback.read(cx).focus_handle(cx);
        self.overlays
            .sync(snapshots, &fallback, self.scope, window, cx);
    }
}

impl Render for Harness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                this.overlays.remember_trigger(window, cx);
            }))
            .on_key_down(cx.listener(|this, _, _, _| this.background_keys += 1))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap_3()
                    .p_4()
                    .child(Input::new(&self.trigger).id("trigger-input").w_64())
                    .child(Input::new(&self.fallback).id("fallback-input").w_64())
                    .child(
                        Button::new("open-lower")
                            .label("Open form…")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.owner.update(cx, |owner, cx| {
                                    owner.open("lower", this.lower_dismissible, window, cx);
                                });
                                this.sync(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("open-buttons")
                            .label("Open decision…")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.owner.update(cx, |owner, cx| {
                                    owner.open("buttons", true, window, cx);
                                });
                                this.sync(window, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(self.overlays.render(700., window, cx))
    }
}

fn initial_scope() -> OverlayScope {
    OverlayScope {
        tab: Some(TabId::new(200)),
        pane: PaneId(1),
        section: SidebarSection::Hosts,
    }
}

fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Harness>) {
    cx.update(gpui_kit::init);
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(800.), px(700.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Harness::new(window, cx)),
        )
        .expect("open overlay test window")
    });
    (handle, view)
}

#[gpui_kit::test]
fn escape_dismisses_only_top_and_restores_nested_then_original_input(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("trigger-input", cx);
        window.input("trigger", cx);
        window.click("open-lower", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("lower-input").focused(), Some(true));
        window.input("lower", cx);
        window.click("open-upper", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("upper-input").focused(), Some(true));
        window.press("escape", cx);
        let owner = view.read(cx).owner.read(cx);
        assert_eq!(owner.dismissals, ["upper"]);
        assert_eq!(owner.prompts.len(), 1);
        assert_eq!(window.find("lower-input").focused(), Some(true));
        window.press("escape", cx);
        assert_eq!(view.read(cx).owner.read(cx).dismissals, ["upper", "lower"]);
        assert_eq!(window.find("trigger-input").focused(), Some(true));
        window.input(" restored", cx);
        assert_eq!(
            window.find("trigger-input").value(),
            Some("trigger restored")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn enter_submits_top_only_and_escape_is_consumed_for_a_required_surface(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |this, _| this.lower_dismissible = false);
        window.render_frame(cx);
        window.click("trigger-input", cx);
        window.click("open-lower", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
        assert!(view.read(cx).owner.read(cx).dismissals.is_empty());
        assert_eq!(view.read(cx).background_keys, 0);
        window.click("open-upper", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
        let owner = view.read(cx).owner.read(cx);
        assert_eq!(owner.submissions.get("upper"), Some(&1));
        assert!(!owner.submissions.contains_key("lower"));
        assert_eq!(view.read(cx).background_keys, 0);
        window.click("lower-pointer-probe", cx);
        assert_eq!(view.read(cx).owner.read(cx).routed_pointer_activations, 0);
    })
    .unwrap();
}

fn fallback_after_scope_change(cx: &mut TestAppContext, scope: OverlayScope) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("trigger-input", cx);
        window.click("open-lower", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |this, cx| {
            this.scope = scope;
            this.sync(window, cx);
        });
        window.render_frame(cx);
        window.press("escape", cx);
        assert_eq!(window.find("fallback-input").focused(), Some(true));
        assert_ne!(window.find("trigger-input").focused(), Some(true));
        window.input("fallback", cx);
        assert_eq!(window.find("fallback-input").value(), Some("fallback"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn workspace_change_restores_current_fallback_instead_of_old_live_trigger(cx: &mut TestAppContext) {
    fallback_after_scope_change(
        cx,
        OverlayScope {
            tab: Some(TabId::new(201)),
            pane: PaneId(2),
            ..initial_scope()
        },
    );
}

#[gpui_kit::test]
fn page_change_restores_current_fallback_instead_of_old_live_trigger(cx: &mut TestAppContext) {
    fallback_after_scope_change(
        cx,
        OverlayScope {
            section: SidebarSection::Settings,
            ..initial_scope()
        },
    );
}

#[gpui_kit::test]
fn stable_surface_reclaims_focus_after_navigation_and_restores_the_new_fallback(
    cx: &mut TestAppContext,
) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("trigger-input", cx);
        window.click("open-lower", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let before = view.read(cx).overlays.top().unwrap();
        view.update(cx, |this, cx| {
            this.scope = OverlayScope {
                tab: Some(TabId::new(202)),
                pane: PaneId(3),
                section: SidebarSection::Settings,
            };
            // Workspace navigation legitimately focuses its destination before
            // synchronizing the still-open, stable owner surface.
            this.fallback.read(cx).focus_handle(cx).focus(window, cx);
            assert!(this.fallback.read(cx).focus_handle(cx).is_focused(window));
            this.sync(window, cx);
        });
        window.render_frame(cx);
        let after = view.read(cx).overlays.top().unwrap();
        assert_eq!(before.stable_key(), after.stable_key());
        assert_eq!(before.revision(), after.revision());
        assert_eq!(window.find("lower-input").focused(), Some(true));
        window.input("still editing", cx);
        assert_eq!(window.find("lower-input").value(), Some("still editing"));
        window.press("escape", cx);
        assert_eq!(window.find("fallback-input").focused(), Some(true));
        assert_ne!(window.find("trigger-input").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn exiting_surface_keeps_its_form_but_cannot_route_pointer_callbacks(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("trigger-input", cx);
        window.click("open-lower", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("captured form", cx);
        window.click("lower-pointer-probe", cx);
        assert_eq!(view.read(cx).owner.read(cx).routed_pointer_activations, 1);
        window.press("escape", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).owner.read(cx).prompts.is_empty());
        assert_eq!(window.find("lower-input").value(), Some("captured form"));
        window.click("lower-pointer-probe", cx);
        assert_eq!(view.read(cx).owner.read(cx).routed_pointer_activations, 1);
        assert_eq!(window.find("trigger-input").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn decision_without_an_input_traps_tab_and_rejects_background_pointer_focus(
    cx: &mut TestAppContext,
) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("trigger-input", cx);
        window.click("open-buttons", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            view.read(cx)
                .overlays
                .top()
                .unwrap()
                .keyboard_focus()
                .is_none()
        );
        assert_ne!(window.find("trigger-input").focused(), Some(true));
        for _ in 0..6 {
            window.press("tab", cx);
            assert_ne!(window.find("trigger-input").focused(), Some(true));
            assert_ne!(window.find("fallback-input").focused(), Some(true));
            assert!(
                window.find("buttons-submit").focused() == Some(true)
                    || window.find("buttons-pointer-probe").focused() == Some(true)
            );
        }
        window.press("shift-tab", cx);
        assert_ne!(window.find("trigger-input").focused(), Some(true));
        window.click("trigger-input", cx);
        assert_ne!(window.find("trigger-input").focused(), Some(true));
        assert!(view.read(cx).owner.read(cx).dismissals.is_empty());
        window.press("escape", cx);
        assert_eq!(window.find("trigger-input").focused(), Some(true));
    })
    .unwrap();
}
