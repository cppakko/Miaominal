use crate::ui::shell::state::SessionSidePanelView;
use crate::ui::{
    components::{SegmentedSwitch, editor_button},
    i18n,
};

use super::super::*;

const WORKSPACE_SIDE_PANEL_GAP: f32 = 12.0;
const WORKSPACE_SIDE_PANEL_SLIDE_OFFSET: f32 = 28.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell::layout) enum WorkspaceSidePanelDock {
    Left,
    Right,
}

pub(in crate::ui::shell::layout) fn workspace_side_panel_render_state(
    desired_visible: bool,
    visible: &mut bool,
    transition: &mut Option<WorkspaceSidePanelTransition>,
    window: &mut Window,
) -> Option<f32> {
    let now = std::time::Instant::now();
    let duration = OVERLAY_ENTER_DURATION;

    match desired_visible {
        true => match transition {
            Some(current) if current.phase == WorkspaceSidePanelTransitionPhase::Exiting => {
                *transition = Some(WorkspaceSidePanelTransition {
                    phase: WorkspaceSidePanelTransitionPhase::Entering,
                    started_at: now,
                    ..*current
                });
            }
            Some(_) => {}
            None => {
                if !*visible {
                    *visible = true;
                    *transition = Some(WorkspaceSidePanelTransition {
                        phase: WorkspaceSidePanelTransitionPhase::Entering,
                        started_at: now,
                        duration,
                    });
                }
            }
        },
        false => {
            if *visible {
                match transition {
                    Some(current)
                        if current.phase == WorkspaceSidePanelTransitionPhase::Entering =>
                    {
                        *transition = Some(WorkspaceSidePanelTransition {
                            phase: WorkspaceSidePanelTransitionPhase::Exiting,
                            started_at: now,
                            ..*current
                        });
                    }
                    None => {
                        *transition = Some(WorkspaceSidePanelTransition {
                            phase: WorkspaceSidePanelTransitionPhase::Exiting,
                            started_at: now,
                            duration,
                        });
                    }
                    _ => {}
                }
            }
        }
    }

    if let Some(current) = *transition {
        let duration_seconds = current.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            *transition = None;
            *visible = current.phase == WorkspaceSidePanelTransitionPhase::Entering;
            return visible.then_some(1.0);
        }

        let elapsed = now.saturating_duration_since(current.started_at);
        let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
        let eased = progress * progress * (3.0 - 2.0 * progress);

        if progress >= 1.0 {
            *transition = None;
            *visible = current.phase == WorkspaceSidePanelTransitionPhase::Entering;
            return visible.then_some(1.0);
        }

        window.request_animation_frame();

        return Some(match current.phase {
            WorkspaceSidePanelTransitionPhase::Entering => eased,
            WorkspaceSidePanelTransitionPhase::Exiting => 1.0 - eased,
        });
    }

    *visible = desired_visible;
    desired_visible.then_some(1.0)
}

pub(in crate::ui::shell::layout) fn render_workspace_side_panel(
    panel: gpui::AnyElement,
    panel_width: f32,
    visibility: f32,
    dock: WorkspaceSidePanelDock,
) -> gpui::AnyElement {
    let wrapper_width = (panel_width + WORKSPACE_SIDE_PANEL_GAP) * visibility;
    let slide_offset = (1.0 - visibility) * WORKSPACE_SIDE_PANEL_SLIDE_OFFSET;
    let opacity = 0.24 + visibility * 0.76;

    let anchored_panel = match dock {
        WorkspaceSidePanelDock::Left => div()
            .absolute()
            .top(px(0.0))
            .left(px(-slide_offset))
            .bottom(px(0.0))
            .w(px(panel_width))
            .opacity(opacity)
            .child(panel),
        WorkspaceSidePanelDock::Right => div()
            .absolute()
            .top(px(0.0))
            .right(px(-slide_offset))
            .bottom(px(0.0))
            .w(px(panel_width))
            .opacity(opacity)
            .child(panel),
    };

    div()
        .relative()
        .h_full()
        .w(px(wrapper_width))
        .min_w(px(0.0))
        .flex_shrink_0()
        .overflow_hidden()
        .child(anchored_panel)
        .into_any_element()
}

impl AppView {
    pub(in crate::ui::shell::layout) fn render_session_workspace_side_panel(
        &self,
        entity: Entity<Self>,
        session_tab_id: usize,
        session: &SessionTabState,
        cx: &App,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let selected_index = match self.panels.session_side_panel_view {
            SessionSidePanelView::Monitor => 0,
            SessionSidePanelView::Snippets => 1,
            SessionSidePanelView::Sftp => 2,
        };
        let switch = SegmentedSwitch::new("session-side-panel-switch")
            .selected_index(selected_index)
            .width(204.0)
            .height(34.0)
            .padding(2.0)
            .item(i18n::string("workspace.panel.monitor.title"))
            .item(i18n::string("snippets.page.snippets"))
            .item("SFTP")
            .on_click({
                let entity = entity.clone();
                move |index, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.panels.session_side_panel_open = true;
                        match index {
                            0 => {
                                this.panels.session_side_panel_view = SessionSidePanelView::Monitor;
                            }
                            1 => {
                                this.panels.session_side_panel_view =
                                    SessionSidePanelView::Snippets;
                            }
                            _ => {
                                this.panels.session_side_panel_view = SessionSidePanelView::Sftp;
                                this.ensure_session_side_panel_sftp_tab(session_tab_id, cx);
                            }
                        }
                        cx.notify();
                    });
                }
            });

        let content = match self.panels.session_side_panel_view {
            SessionSidePanelView::Monitor => {
                self.render_session_monitor_panel(entity.clone(), session)
            }
            SessionSidePanelView::Snippets => {
                self.render_session_snippets_panel(entity.clone(), cx)
            }
            SessionSidePanelView::Sftp => {
                self.render_session_sftp_panel(entity.clone(), session_tab_id)
            }
        };

        card_surface(roles.surface_container, 16.0)
            .id("session-workspace-side-panel")
            .w(px(SESSION_MONITOR_PANEL_WIDTH))
            .h_full()
            .flex_shrink_0()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .child(
                v_flex()
                    .size_full()
                    .overflow_hidden()
                    .child(div().flex_1().min_h(px(0.0)).child(content))
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(56.0))
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .px_3()
                            .child(switch),
                    ),
            )
            .into_any_element()
    }

    fn render_session_sftp_panel(
        &self,
        entity: Entity<Self>,
        session_tab_id: usize,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );

        if let Some(tab_id) = self.session_side_panel_sftp_tab_id()
            && let Some(sftp_tab) = self
                .workspace_state
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp)
        {
            return div()
                .id("session-sftp-panel-content")
                .size_full()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .px_3()
                .pb_3()
                .child(self.render_sftp_remote_browser_panel(entity, tab_id, sftp_tab))
                .into_any_element();
        }

        v_flex()
            .id("session-sftp-panel-content")
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_3()
            .child(
                div()
                    .size(px(44.0))
                    .rounded(px(12.0))
                    .bg(rgb(roles.surface))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(roles.primary))
                    .child(Icon::new(AppIcon::FolderSymlink).size(px(22.0))),
            )
            .child(
                div()
                    .max_w(px(280.0))
                    .text_center()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(text_muted))
                    .child(i18n::string("sftp_browser.empty.remote")),
            )
            .child(editor_button(
                i18n::string("workspace.menu.open_sftp_tab"),
                false,
                true,
                move |_, cx| {
                    entity.update(cx, |this, cx| {
                        this.ensure_session_side_panel_sftp_tab(session_tab_id, cx);
                    });
                },
            ))
            .into_any_element()
    }
}
