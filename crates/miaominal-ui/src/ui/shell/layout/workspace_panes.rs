use crate::ui::i18n;
use gpui::StatefulInteractiveElement;
use miaominal_settings::TerminalRightClickBehavior;

use super::super::metrics::TERMINAL_PANEL_BORDER;
use super::super::*;
use super::chrome::{status_indicator, tab_status_indicator_color};
use super::workspace_terminal_menu::{build_terminal_context_menu, terminal_pane_surface_id};

fn pane_drop_zone_style(
    style: gpui::StyleRefinement,
    zone: super::super::panes::PaneTabDropZone,
) -> gpui::StyleRefinement {
    let roles = miaominal_settings::current_theme().material.roles;
    let mut refined = style;
    refined.background = Some(color_with_alpha(roles.primary, 0x33).into());
    refined.border_color = Some(rgb(roles.primary).into());
    match zone {
        super::super::panes::PaneTabDropZone::Center => {
            refined.border_widths.top = Some(px(2.0).into());
            refined.border_widths.right = Some(px(2.0).into());
            refined.border_widths.bottom = Some(px(2.0).into());
            refined.border_widths.left = Some(px(2.0).into());
        }
        super::super::panes::PaneTabDropZone::Up => {
            refined.border_widths.bottom = Some(px(2.0).into());
        }
        super::super::panes::PaneTabDropZone::Down => {
            refined.border_widths.top = Some(px(2.0).into());
        }
        super::super::panes::PaneTabDropZone::Left => {
            refined.border_widths.right = Some(px(2.0).into());
        }
        super::super::panes::PaneTabDropZone::Right => {
            refined.border_widths.left = Some(px(2.0).into());
        }
    }
    refined
}

impl AppView {
    pub(in crate::ui::shell::layout) fn render_pane_layout(
        &mut self,
        layout: &PaneLayout,
        path: &[usize],
        valid_source_ids: &[usize],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match layout {
            PaneLayout::Leaf(pane_id) => {
                let embedded_in_split = !path.is_empty();
                let body = if *pane_id == self.workspace_state.workspace.active_pane_id {
                    self.render_active_pane_surface(embedded_in_split, window, cx)
                } else {
                    self.render_parked_pane_surface(*pane_id, embedded_in_split, cx)
                };
                let content = if path.is_empty() {
                    body
                } else {
                    let is_active = *pane_id == self.workspace_state.workspace.active_pane_id;
                    let roles = miaominal_settings::current_theme().material.roles;
                    let header = self.render_pane_header(*pane_id);
                    div()
                        .rounded(px(16.0))
                        .border(px(TERMINAL_PANEL_BORDER))
                        .border_color(if is_active {
                            rgb(roles.primary)
                        } else {
                            color_with_alpha(roles.primary, 0x00)
                        })
                        .overflow_hidden()
                        .bg(rgb(roles.background))
                        .flex()
                        .flex_col()
                        .size_full()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .child(header)
                        .child(
                            div()
                                .flex_grow(1.0)
                                .flex_shrink(1.0)
                                .flex_basis(gpui::relative(1.0))
                                .min_w(px(0.0))
                                .min_h(px(0.0))
                                .child(body),
                        )
                        .into_any_element()
                };
                let content = self.render_split_animated_pane(path, content);

                self.render_pane_drop_surface(*pane_id, content, valid_source_ids, cx)
            }
            PaneLayout::Split {
                axis,
                children,
                flexes,
            } => {
                let mut container = match axis {
                    super::super::workspace::SplitAxis::Horizontal => div().flex().flex_row(),
                    super::super::workspace::SplitAxis::Vertical => div().flex().flex_col(),
                };
                container = container.size_full().min_w(px(0.0)).min_h(px(0.0));
                for (i, child) in children.iter().enumerate() {
                    let mut child_path = path.to_vec();
                    child_path.push(i);
                    let flex = flexes.get(i).copied().unwrap_or(1.0).max(0.001);
                    let child_element =
                        self.render_pane_layout(child, &child_path, valid_source_ids, window, cx);
                    let wrapped = div()
                        .flex_grow(1.0)
                        .flex_shrink(1.0)
                        .flex_basis(gpui::relative(flex))
                        .h_full()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .child(child_element);
                    container = container.child(wrapped);
                    if i + 1 < children.len() {
                        let mut bar_path = path.to_vec();
                        bar_path.push(i);
                        container = container.child(self.render_split_bar(*axis, bar_path, cx));
                    }
                }
                container.into_any_element()
            }
        }
    }

    fn render_split_animated_pane(
        &self,
        path: &[usize],
        content: gpui::AnyElement,
    ) -> gpui::AnyElement {
        let Some(animation) = self.workspace_state.workspace.pane_split_animation.as_ref() else {
            return content;
        };
        let Some((&child_index, parent_path)) = path.split_last() else {
            return content;
        };
        if parent_path != animation.path.as_slice() || child_index != animation.new_child_index {
            return content;
        }

        let duration_seconds = animation.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            return content;
        }

        let progress =
            (animation.started_at.elapsed().as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
        let eased = progress * progress * (3.0 - 2.0 * progress);
        let offset = match animation.kind {
            super::super::panes::PaneSplitAnimationKind::Opening => (1.0 - eased) * 14.0,
            super::super::panes::PaneSplitAnimationKind::Closing => eased * 14.0,
        };
        let animated_is_primary = animation.new_child_index == animation.child_index;
        let (left_offset, top_offset) = match animation.axis {
            super::super::workspace::SplitAxis::Horizontal => {
                let signed = if animated_is_primary { -offset } else { offset };
                (signed, 0.0)
            }
            super::super::workspace::SplitAxis::Vertical => {
                let signed = if animated_is_primary { -offset } else { offset };
                (0.0, signed)
            }
        };
        let opacity = match animation.kind {
            super::super::panes::PaneSplitAnimationKind::Opening => 0.42 + eased * 0.58,
            super::super::panes::PaneSplitAnimationKind::Closing => 1.0 - eased * 0.62,
        };

        div()
            .relative()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .opacity(opacity)
            .left(px(left_offset))
            .top(px(top_offset))
            .child(content)
            .into_any_element()
    }

    fn render_pane_drop_surface(
        &self,
        pane_id: super::super::panes::PaneId,
        content: gpui::AnyElement,
        valid_source_ids: &[usize],
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if valid_source_ids.is_empty() {
            return content;
        }

        div()
            .relative()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .child(content)
            .child(self.render_pane_drop_targets(pane_id, valid_source_ids, cx))
            .into_any_element()
    }

    fn render_pane_drop_targets(
        &self,
        pane_id: super::super::panes::PaneId,
        valid_source_ids: &[usize],
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use super::super::panes::PaneTabDropZone;

        let entity = cx.entity();
        let center_enabled = self
            .pane_tab_index(pane_id)
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)
            .is_some();
        let edge_inset = gpui::relative(0.24);

        let render_zone = |base: gpui::Div,
                           zone: PaneTabDropZone,
                           accepts: bool,
                           ids: Vec<usize>,
                           entity: Entity<AppView>| {
            let ids_for_drag = ids.clone();
            let ids_for_drop = ids;
            let entity_for_drop = entity.clone();

            base.drag_over::<DraggedTab>(move |style, payload: &DraggedTab, _, _| {
                if !accepts || !ids_for_drag.contains(&payload.source_tab_id) {
                    return style;
                }
                pane_drop_zone_style(style, zone)
            })
            .on_drop::<DraggedTab>(move |payload: &DraggedTab, window, cx| {
                if !accepts || !ids_for_drop.contains(&payload.source_tab_id) {
                    return;
                }
                let source_tab_id = payload.source_tab_id;
                entity_for_drop.update(cx, |this, cx| {
                    this.handle_pane_tab_drop(source_tab_id, pane_id, zone, window, cx);
                });
            })
            .into_any_element()
        };

        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .child(render_zone(
                div().absolute().top_0().left_0().right_0().h(edge_inset),
                PaneTabDropZone::Up,
                true,
                valid_source_ids.to_vec(),
                entity.clone(),
            ))
            .child(render_zone(
                div().absolute().bottom_0().left_0().right_0().h(edge_inset),
                PaneTabDropZone::Down,
                true,
                valid_source_ids.to_vec(),
                entity.clone(),
            ))
            .child(render_zone(
                div()
                    .absolute()
                    .top(edge_inset)
                    .bottom(edge_inset)
                    .left_0()
                    .w(edge_inset),
                PaneTabDropZone::Left,
                true,
                valid_source_ids.to_vec(),
                entity.clone(),
            ))
            .child(render_zone(
                div()
                    .absolute()
                    .top(edge_inset)
                    .bottom(edge_inset)
                    .right_0()
                    .w(edge_inset),
                PaneTabDropZone::Right,
                true,
                valid_source_ids.to_vec(),
                entity.clone(),
            ))
            .child(render_zone(
                div()
                    .absolute()
                    .top(edge_inset)
                    .right(edge_inset)
                    .bottom(edge_inset)
                    .left(edge_inset),
                PaneTabDropZone::Center,
                center_enabled,
                valid_source_ids.to_vec(),
                entity,
            ))
            .into_any_element()
    }

    fn render_pane_header(&self, pane_id: super::super::panes::PaneId) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let is_active = pane_id == self.workspace_state.workspace.active_pane_id;
        let tab_index = if is_active {
            self.workspace_state.workspace.active_tab
        } else {
            self.parked_pane(pane_id).and_then(|p| p.active_tab)
        };
        let (label, status_color) = tab_index
            .and_then(|idx| self.workspace_state.tabs.get(idx))
            .map(|tab| {
                let label = tab
                    .as_session()
                    .and_then(|s| {
                        self.data
                            .sessions
                            .iter()
                            .find(|p| p.id == s.profile_id)
                            .map(|p| p.name.clone())
                    })
                    .unwrap_or_else(|| tab.title.to_string());
                let has_activity = tab.as_session().is_some_and(|session| session.has_activity);
                (label, tab_status_indicator_color(tab, has_activity))
            })
            .unwrap_or_else(|| (i18n::string("session.workspace.empty_tab_label"), None));

        div()
            .flex_shrink_0()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .when_some(status_color, |this, color| {
                this.child(status_indicator(color, 7.0))
            })
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .text_color(rgb(if is_active {
                        roles.on_surface
                    } else {
                        text_muted
                    }))
                    .when(is_active, |this| this.font_weight(gpui::FontWeight::MEDIUM))
                    .child(SharedString::from(label)),
            )
            .into_any_element()
    }

    fn render_split_bar(
        &self,
        axis: super::super::workspace::SplitAxis,
        path: Vec<usize>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use super::super::panes::{PaneSplitDragMarker, PaneSplitDragState};
        use super::super::workspace::SplitAxis;
        let Some((&child_index, parent_path)) = path.split_last() else {
            return div().into_any_element();
        };
        let parent_path: Vec<usize> = parent_path.to_vec();
        let parent_path_for_drag = parent_path.clone();
        let parent_path_for_move = parent_path.clone();
        let marker = PaneSplitDragMarker {
            path: parent_path_for_drag.clone(),
            child_index,
            axis,
        };

        let bar_id = SharedString::from(format!(
            "split-bar-{}-{}",
            parent_path
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join("-"),
            child_index
        ));

        const PANE_GAP: f32 = 8.0;
        let is_dragging = self
            .workspace_state
            .workspace
            .pane_split_drag
            .as_ref()
            .is_some_and(|drag| {
                drag.axis == axis && drag.child_index == child_index && drag.path == parent_path
            });

        let mut bar = div().id(bar_id).flex_shrink_0().occlude();
        bar = match axis {
            SplitAxis::Horizontal => bar.w(px(PANE_GAP)).h_full().cursor_col_resize(),
            SplitAxis::Vertical => bar.h(px(PANE_GAP)).w_full().cursor_row_resize(),
        };

        let path_for_start = parent_path.clone();
        bar.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                let path = path_for_start.clone();
                let (flex_a, flex_b) = {
                    let mut node = &this.workspace_state.workspace.pane_layout;
                    for &i in &path {
                        match node {
                            super::super::workspace::PaneLayout::Split { children, .. } => {
                                let Some(next) = children.get(i) else {
                                    return;
                                };
                                node = next;
                            }
                            _ => return,
                        }
                    }
                    if let super::super::workspace::PaneLayout::Split { flexes, .. } = node
                        && let (Some(a), Some(b)) =
                            (flexes.get(child_index), flexes.get(child_index + 1))
                    {
                        (*a, *b)
                    } else {
                        return;
                    }
                };
                let initial_pointer = match axis {
                    SplitAxis::Horizontal => f32::from(event.position.x),
                    SplitAxis::Vertical => f32::from(event.position.y),
                };
                let container_size = this.split_container_size(&path, axis, window);
                this.workspace_state.workspace.pane_split_drag = Some(PaneSplitDragState {
                    path,
                    child_index,
                    axis,
                    initial_pointer,
                    initial_flex_a: flex_a,
                    initial_flex_b: flex_b,
                    container_size,
                });
                cx.notify();
            }),
        )
        .hover(move |this| {
            if is_dragging {
                this
            } else {
                match axis {
                    SplitAxis::Horizontal => this.cursor_col_resize(),
                    SplitAxis::Vertical => this.cursor_row_resize(),
                }
            }
        })
        .on_drag(marker, |m, _offset, _window, cx| cx.new(|_| m.clone()))
        .on_drag_move::<PaneSplitDragMarker>(cx.listener(
            move |this, event: &gpui::DragMoveEvent<PaneSplitDragMarker>, _window, cx| {
                let _ = &parent_path_for_move; // capture for closure type
                let Some(drag) = this.workspace_state.workspace.pane_split_drag.clone() else {
                    return;
                };
                let pointer = match drag.axis {
                    SplitAxis::Horizontal => f32::from(event.event.position.x),
                    SplitAxis::Vertical => f32::from(event.event.position.y),
                };
                let delta_px = pointer - drag.initial_pointer;
                let delta_flex = if drag.container_size > 0.0 {
                    delta_px / drag.container_size
                } else {
                    0.0
                };
                let new_a = (drag.initial_flex_a + delta_flex).clamp(0.05, 0.95);
                let new_b = (drag.initial_flex_b - delta_flex).clamp(0.05, 0.95);
                this.apply_split_flex_delta(&drag.path, drag.child_index, new_a, new_b);
                cx.notify();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _event: &MouseUpEvent, _, cx| {
                if this
                    .workspace_state
                    .workspace
                    .pane_split_drag
                    .take()
                    .is_some()
                {
                    cx.notify();
                }
            }),
        )
        .into_any_element()
    }

    fn render_parked_pane_surface(
        &mut self,
        pane_id: super::super::panes::PaneId,
        embedded_in_split: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let parked_tab_index = self
            .parked_pane(pane_id)
            .and_then(|parked| parked.active_tab);
        let cell_width = self
            .parked_pane(pane_id)
            .map(|p| p.terminal_cell_width)
            .unwrap_or(
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_cell_width,
            );
        let line_height = self
            .parked_pane(pane_id)
            .map(|p| p.terminal_line_height)
            .unwrap_or(
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_line_height,
            );

        let placeholder = parked_tab_index.and_then(|active| {
            self.workspace_state
                .tabs
                .get(active)
                .and_then(|tab| self.render_session_placeholder(tab, !embedded_in_split, cx))
        });
        let history_banner = parked_tab_index.and_then(|active| {
            self.workspace_state
                .tabs
                .get(active)
                .and_then(|tab| self.render_session_history_banner(tab, cx))
        });

        let has_terminal = parked_tab_index
            .and_then(|active| self.workspace_state.tabs.get(active))
            .and_then(TabState::as_session)
            .is_some();

        let weak = cx.entity().downgrade();
        let show_scrollbar = self.terminal_scrollbar_visible(pane_id);
        let terminal_settings = self.settings_store.settings().clone();

        let pane_surface = div()
            .id(terminal_pane_surface_id(pane_id))
            .relative()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .rounded(px(16.0))
            .overflow_hidden()
            .bg(rgb(roles.background))
            .text_color(rgb(roles.on_surface))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                    this.set_active_pane(pane_id, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                    this.set_active_pane(pane_id, window, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseUpEvent, window, cx| {
                    let settings = miaominal_settings::current_settings();
                    let force_context_menu =
                        settings.terminal_shift_right_click_context_menu && event.modifiers.shift;
                    if force_context_menu
                        || settings.terminal_right_click_behavior
                            != TerminalRightClickBehavior::CopySelectionOrPaste
                    {
                        return;
                    }

                    this.set_active_pane(pane_id, window, cx);
                    this.handle_terminal_mouse_up(event, cx);
                }),
            )
            .child(if has_terminal {
                div()
                    .size_full()
                    .px_4()
                    .when(!embedded_in_split, |this| this.pt_4())
                    .child(render_terminal_canvas_for_pane(
                        None,
                        cell_width,
                        line_height,
                        weak,
                        pane_id,
                        show_scrollbar,
                    ))
                    .into_any_element()
            } else {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(text_muted))
                    .child(i18n::string("session.workspace.click_to_focus"))
                    .into_any_element()
            })
            .when_some(history_banner, |this, banner| this.child(banner))
            .when_some(placeholder, |this, placeholder| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0()
                        .child(placeholder),
                )
            });

        if terminal_settings
            .terminal_right_click_behavior
            .uses_context_menu()
            || terminal_settings.terminal_shift_right_click_context_menu
        {
            let entity = cx.entity();
            pane_surface
                .context_menu({
                    let terminal_settings = terminal_settings.clone();
                    move |menu, window, cx| {
                        if !(terminal_settings
                            .terminal_right_click_behavior
                            .uses_context_menu()
                            || terminal_settings.terminal_shift_right_click_context_menu
                                && window.modifiers().shift)
                        {
                            return menu;
                        }

                        let view = entity.read(cx);
                        let has_selection = view
                            .workspace_state
                            .workspace
                            .parked_panes
                            .get(&pane_id)
                            .and_then(|parked| parked.active_tab)
                            .and_then(|index| view.workspace_state.tabs.get(index))
                            .and_then(TabState::as_session)
                            .is_some_and(|session| session.terminal.has_selection());

                        build_terminal_context_menu(
                            menu,
                            entity.clone(),
                            has_selection,
                            Some(pane_id),
                            window,
                            cx,
                        )
                    }
                })
                .into_any_element()
        } else {
            pane_surface.into_any_element()
        }
    }

    fn render_active_pane_surface(
        &mut self,
        embedded_in_split: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let active_index = self.workspace_state.workspace.active_tab;
        let terminal_focused = self
            .workspace_state
            .workspace
            .active_pane
            .terminal_focus
            .is_focused(window);
        let cell_width = terminal_cell_width(window);
        let line_height = terminal_line_height(window);
        let active_pane_id = self.workspace_state.workspace.active_pane_id;
        let show_scrollbar = self.terminal_scrollbar_visible(active_pane_id);

        let pane_surface = div()
            .id(terminal_pane_surface_id(active_pane_id))
            .size_full()
            .relative()
            .rounded(px(16.0))
            .border_color(rgb(if terminal_focused && active_index.is_some() {
                roles.primary
            } else {
                roles.outline_variant
            }))
            .bg(rgb(roles.surface_container_highest))
            .overflow_hidden()
            .when_some(active_index, |this, index| {
                let tab = &self.workspace_state.tabs[index];
                if tab.as_session().is_none() {
                    return this;
                }
                let placeholder = self.render_session_placeholder(tab, !embedded_in_split, cx);
                let history_banner = self.render_session_history_banner(tab, cx);
                let tab_id = tab.id;
                let hovered_link = self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_hovered_link
                    .clone()
                    .filter(|hovered| hovered.tab_id == tab_id);
                let show_link_cursor = hovered_link.is_some()
                    && self
                        .workspace_state
                        .workspace
                        .active_pane
                        .terminal_link_open_modifier;
                let weak = cx.entity().downgrade();

                let terminal_surface = div()
                    .id(SharedString::from(format!("terminal-output-{}", tab_id)))
                    .track_focus(&self.workspace_state.workspace.active_pane.terminal_focus)
                    .size_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .rounded(px(16.0))
                    .overflow_hidden()
                    .px_4()
                    .when(!embedded_in_split, |this| this.pt_4())
                    .bg(rgb(roles.background))
                    .text_color(rgb(roles.on_surface))
                    .when(show_link_cursor, |this| this.cursor_pointer())
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        this.handle_terminal_hover(*hovered, cx);
                    }))
                    .on_modifiers_changed(cx.listener(
                        move |this, event: &ModifiersChangedEvent, _, cx| {
                            this.handle_terminal_modifiers_changed(event, cx);
                        },
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            window.focus(
                                &this.workspace_state.workspace.active_pane.terminal_focus,
                                cx,
                            );
                            this.handle_terminal_mouse_down(event, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        this.handle_terminal_mouse_move(event, cx);
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                            this.handle_terminal_mouse_up(event, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.handle_terminal_mouse_down(event, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Middle,
                        cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                            this.handle_terminal_mouse_up(event, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            window.focus(
                                &this.workspace_state.workspace.active_pane.terminal_focus,
                                cx,
                            );
                            this.handle_terminal_mouse_down(event, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                            this.handle_terminal_mouse_up(event, cx);
                        }),
                    )
                    .on_key_down(cx.listener(Self::handle_terminal_key_down))
                    .on_key_up(cx.listener(Self::handle_terminal_key_up))
                    .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
                        this.handle_terminal_scroll_wheel(event, line_height, cx);
                    }))
                    .child(render_terminal_canvas_for_pane(
                        hovered_link,
                        cell_width,
                        line_height,
                        weak,
                        active_pane_id,
                        show_scrollbar,
                    ))
                    .when_some(
                        self.advance_terminal_search_overlay(window),
                        |this, visibility| {
                            this.child(self.render_terminal_search_overlay(visibility, cx))
                        },
                    );

                let terminal_stack = div()
                    .flex_1()
                    .min_h(px(0.0))
                    .relative()
                    .child(terminal_surface)
                    .when_some(placeholder, |this, placeholder| {
                        this.child(
                            div()
                                .absolute()
                                .top_0()
                                .right_0()
                                .bottom_0()
                                .left_0()
                                .child(placeholder),
                        )
                    })
                    .when_some(history_banner, |this, banner| this.child(banner));

                this.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .min_h(px(0.0))
                        .child(terminal_stack),
                )
            })
            .when(active_index.is_none(), |this| {
                this.child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            v_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(
                                            miaominal_settings::FontSize::SectionTitle.scaled(),
                                        )
                                        .text_color(rgb(roles.on_surface))
                                        .child(i18n::string("workspace.empty.open_ssh_tab_title")),
                                )
                                .child(
                                    div()
                                        .text_size(miaominal_settings::FontSize::Input.scaled())
                                        .text_color(rgb(text_muted))
                                        .child(i18n::string("workspace.empty.open_ssh_tab_body")),
                                ),
                        ),
                )
            });

        let terminal_settings = self.settings_store.settings().clone();
        if active_index.is_some()
            && (terminal_settings
                .terminal_right_click_behavior
                .uses_context_menu()
                || terminal_settings.terminal_shift_right_click_context_menu)
        {
            let entity = cx.entity();
            pane_surface
                .context_menu({
                    let terminal_settings = terminal_settings.clone();
                    move |menu, window, cx| {
                        if !(terminal_settings
                            .terminal_right_click_behavior
                            .uses_context_menu()
                            || terminal_settings.terminal_shift_right_click_context_menu
                                && window.modifiers().shift)
                        {
                            return menu;
                        }

                        let view = entity.read(cx);
                        let has_selection = view
                            .workspace_state
                            .workspace
                            .active_tab
                            .and_then(|index| view.workspace_state.tabs.get(index))
                            .and_then(TabState::as_session)
                            .is_some_and(|session| session.terminal.has_selection());

                        build_terminal_context_menu(
                            menu,
                            entity.clone(),
                            has_selection,
                            None,
                            window,
                            cx,
                        )
                    }
                })
                .into_any_element()
        } else {
            pane_surface.into_any_element()
        }
    }
}
