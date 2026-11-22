use super::super::*;
use crate::ui::components::animation::CONTAINER_TRANSITION_DURATION;
use crate::ui::components::shell_tokens::{ShellColors, ShellMetrics, space};
use gpui_kit::component::{
    Selectable, button::ButtonCustomVariant, scroll::ScrollableElement as _,
};
use gpui_kit::{Pixels, SpringConfig, SpringState, ease_in_out};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

/// A spatial spring with a damping ratio of about 0.9: it settles in roughly
/// 300 ms with a barely visible overshoot and keeps its velocity when a new
/// page interrupts it.
const DOCK_ACTION_SPRING: SpringConfig = SpringConfig::new(800.0, 51.0, 1.0);
/// Settling tolerance in logical pixels.
const DOCK_ACTION_SPRING_EPSILON: f32 = 0.5;
const DOCK_ACTION_ENTER_OFFSET: f32 = 8.0;
/// Longest step, in seconds, the dock spring takes between two renders.
const DOCK_ACTION_MAX_FRAME_DELTA: f32 = 1.0 / 30.0;

/// The page command shown at the trailing end of the bottom navigation dock.
pub(in crate::ui::shell) struct DockAction {
    /// Stable per-page key; a new key replays the command's enter animation.
    pub key: &'static str,
    pub element: AnyElement,
}

/// Motion state of the dock's page-command slot.
///
/// It lives on the view rather than in GPUI element state because the dock is
/// rendered under different parents while a page transition runs and after it
/// settles; element state would restart the motion at each switch.
#[derive(Default)]
pub(in crate::ui::shell) struct DockActionMotion {
    /// Natural width of the current command, measured after layout.
    measured: Option<Pixels>,
    width: Option<SpringState>,
    updated_at: Option<Instant>,
    /// Key of the shown command and when it appeared (enter fade/slide).
    key: Option<&'static str>,
    entered_at: Option<Instant>,
}

impl DockActionMotion {
    /// Advances the motion to `now` and returns the slot width (None while the
    /// first command is unmeasured) and the command's enter progress.
    fn advance(
        &mut self,
        key: Option<&'static str>,
        now: Instant,
        reduce_motion: bool,
    ) -> (Option<f32>, f32) {
        if key != self.key {
            self.key = key;
            // The first command of the session appears in place.
            self.entered_at = (self.width.is_some() && key.is_some()).then_some(now);
        }
        let target = if key.is_some() {
            self.measured.map(f32::from)
        } else {
            Some(0.0)
        };
        // Renders also stop while the dock rests, so the time since the last
        // render is not a frame delta. Cap it so a new target starts moving
        // from rest instead of jumping straight to its end.
        let elapsed = self.updated_at.map_or(0.0, |at| {
            now.saturating_duration_since(at)
                .as_secs_f32()
                .min(DOCK_ACTION_MAX_FRAME_DELTA)
        });
        self.updated_at = Some(now);
        self.width = match (self.width, target) {
            (_, None) => None,
            (None, Some(target)) => Some(SpringState {
                position: target,
                velocity: 0.0,
            }),
            (Some(state), Some(target)) => {
                let state = DOCK_ACTION_SPRING.step(state, target, elapsed);
                if reduce_motion
                    || DOCK_ACTION_SPRING.is_settled(state, target, DOCK_ACTION_SPRING_EPSILON)
                {
                    Some(SpringState {
                        position: target,
                        velocity: 0.0,
                    })
                } else {
                    Some(state)
                }
            }
        };
        let enter = match self.entered_at {
            Some(at) if !reduce_motion => ease_in_out(
                (now.saturating_duration_since(at).as_secs_f32()
                    / CONTAINER_TRANSITION_DURATION.as_secs_f32())
                .clamp(0.0, 1.0),
            ),
            _ => 1.0,
        };
        if enter >= 1.0 {
            self.entered_at = None;
        }
        (self.width.map(|state| state.position.max(0.0)), enter)
    }

    fn is_animating(&self) -> bool {
        let width_moving = match (self.width, self.key.map(|_| self.measured)) {
            (Some(state), Some(Some(target))) => state.position != f32::from(target),
            (Some(state), None) => state.position != 0.0,
            _ => false,
        };
        width_moving || self.entered_at.is_some()
    }
}

fn sidebar_item(
    section: SidebarSection,
    is_selected: bool,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let colors = ShellColors::current();
    let background = if is_selected {
        colors.selected
    } else {
        colors.navigation_bar
    };
    let foreground = if is_selected {
        colors.on_selected
    } else {
        colors.on_navigation_bar
    };
    let item_id = match section {
        SidebarSection::Hosts => "sidebar-hosts",
        SidebarSection::Keychain => "sidebar-keychain",
        SidebarSection::PortForwarding => "sidebar-forwarding",
        SidebarSection::Snippets => "sidebar-snippets",
        SidebarSection::KnownHosts => "sidebar-known-hosts",
        SidebarSection::Settings => "sidebar-settings",
    };
    Button::new(item_id)
        .ghost()
        .selected(is_selected)
        .accessibility_label(section.title())
        .tooltip(section.title())
        .w(space(ShellMetrics::NAV_INDICATOR_WIDTH))
        .h(space(ShellMetrics::NAV_INDICATOR_HEIGHT))
        .rounded_full()
        .custom(
            ButtonCustomVariant::new(cx)
                .color(background)
                .foreground(foreground)
                .hover(background.blend(foreground.opacity(0.08)))
                .active(if is_selected {
                    background
                } else {
                    background.blend(foreground.opacity(0.12))
                }),
        )
        .child(section.icon())
        .on_click(move |_, window, cx| on_click(window, cx))
}

/// Bottom navigation dock. When the current page has a primary command (for
/// example "New host"), it is docked at the trailing end of the same pill so
/// navigation and the page's main action form one control surface.
pub(in crate::ui::shell) fn render_sidebar(
    app: &AppView,
    command_source: Entity<SettingsController>,
    primary_action: Option<DockAction>,
    cx: &App,
) -> impl IntoElement {
    let colors = ShellColors::current();
    div()
        .w_full()
        .h(space(ShellMetrics::NAV_BAR_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .px(space(8.0))
        .child(
            h_flex()
                .id("bottom-navigation-dock")
                .occlude()
                .max_w_full()
                .min_w_0()
                .h(space(64.0))
                .items_center()
                .rounded(space(999.0))
                .bg(colors.navigation_bar)
                .p(space(8.0))
                .child(
                    h_flex()
                        .min_w_0()
                        .h_full()
                        .items_center()
                        .gap(space(8.0))
                        .overflow_x_scrollbar()
                        .children(SidebarSection::all().into_iter().map({
                            let entity = command_source.clone();
                            move |section| {
                                let entity = entity.clone();
                                div()
                                    .w(space(56.0))
                                    .h(space(48.0))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .child(sidebar_item(
                                        section,
                                        app.shell.shell_state.sidebar_section == section,
                                        cx,
                                        move |_, cx| {
                                            entity.update(cx, |controller, cx| {
                                                controller.emit(
                                                    AppCommand::SidebarSectionRequested(section),
                                                    cx,
                                                );
                                            })
                                        },
                                    ))
                            }
                        }))
                        .child(
                            div()
                                .w(space(56.0))
                                .h(space(48.0))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .child(sidebar_item(
                                    SidebarSection::Settings,
                                    app.shell.shell_state.sidebar_section
                                        == SidebarSection::Settings,
                                    cx,
                                    move |_, cx| {
                                        command_source.update(cx, |controller, cx| {
                                            controller.emit(
                                                AppCommand::SidebarSectionRequested(
                                                    SidebarSection::Settings,
                                                ),
                                                cx,
                                            );
                                        })
                                    },
                                )),
                        ),
                )
                .child(render_dock_action_slot(
                    primary_action,
                    app.shell.dock_action_motion.clone(),
                    cx,
                )),
        )
}

/// The trailing page-command slot of the bottom navigation dock.
///
/// Its width springs toward the measured natural width of the current command,
/// so the dock grows, shrinks, or collapses smoothly when the page changes,
/// while the incoming command fades and slides in. Reduced motion snaps to the
/// end state.
fn render_dock_action_slot(
    action: Option<DockAction>,
    motion: Rc<RefCell<DockActionMotion>>,
    cx: &App,
) -> AnyElement {
    let colors = ShellColors::current();
    let (width, enter) = motion.borrow_mut().advance(
        action.as_ref().map(|action| action.key),
        Instant::now(),
        cx.reduce_motion(),
    );
    let content = action.map(|DockAction { element, .. }| {
        h_flex()
            .flex_none()
            .h_full()
            .items_center()
            // The dock gap, the divider, and the trailing inset belong to the
            // slot so that an empty slot collapses to exactly zero width.
            .pl(space(8.0))
            .pr(space(4.0))
            .gap(space(8.0))
            .child(
                // Separates destinations from the page command without
                // breaking the dock into two surfaces.
                div()
                    .flex_shrink_0()
                    .w(px(1.0))
                    .h(space(24.0))
                    .bg(colors.on_navigation_bar.opacity(0.24)),
            )
            .child(
                div()
                    .relative()
                    .flex_shrink_0()
                    .opacity(enter)
                    .left(px((1.0 - enter) * DOCK_ACTION_ENTER_OFFSET))
                    .child(element),
            )
    });
    let has_content = content.is_some();

    h_flex()
        .flex_shrink_0()
        .h_full()
        .items_center()
        .overflow_hidden()
        // Until the first command is measured it keeps its natural width.
        .when_some(width, |slot, width| slot.w(px(width)))
        .children(content)
        .on_children_prepainted(move |bounds, window, _| {
            let mut motion = motion.borrow_mut();
            if has_content
                && let Some(measured) = bounds.first().map(|bounds| bounds.size.width)
                && motion.measured != Some(measured)
            {
                motion.measured = Some(measured);
                window.refresh();
            }
            if motion.is_animating() {
                window.request_animation_frame();
            }
        })
        .into_any_element()
}
