use super::shell_tokens::ShellMetrics;
use crate::ui::contracts::navigation::ProfileViewMode;
use gpui_kit::base::motion::{Transition, transition};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    AnyElement, App, AvailableSpace, InteractiveElement as _, IntoElement, ParentElement,
    RenderOnce, SharedString, Styled, TestSupportExt as _, Window, canvas, div, point, px, size,
};

/// Shared motion policy for the selector and the collection. The framework keeps
/// interrupted transitions continuous and honors the platform motion preference.
pub(super) fn view_mode_progress(
    id: &'static str,
    mode: ProfileViewMode,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    let motion = cx.theme().motion_tokens();
    let policy = Transition::new(motion.duration_slow).easing(motion.easing_move.clone());
    transition(
        (id, "view-mode"),
        if mode == ProfileViewMode::List {
            1.0
        } else {
            0.0
        },
        policy,
        window,
        cx,
    )
}

pub(in crate::ui) fn animated_card_collection(
    id: &'static str,
    mode: ProfileViewMode,
    columns: u16,
    list_gap: f32,
    items: Vec<(SharedString, AnyElement)>,
) -> impl IntoElement {
    AnimatedCardCollection {
        id,
        mode,
        columns,
        list_gap,
        items,
    }
}

#[derive(IntoElement)]
struct AnimatedCardCollection {
    id: &'static str,
    mode: ProfileViewMode,
    columns: u16,
    list_gap: f32,
    items: Vec<(SharedString, AnyElement)>,
}

impl RenderOnce for AnimatedCardCollection {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let progress = view_mode_progress(self.id, self.mode, window, cx);
        let columns = usize::from(self.columns.max(1));
        let count = self.items.len();
        let grid_rows = count.div_ceil(columns);
        let scale = f32::from(window.rem_size()) / 14.0;
        let device_scale = window.scale_factor();
        // Match GPUI grid gap snapping before distributing equal-width tracks.
        let snap = move |value: f32| (value * device_scale - 0.5).ceil() / device_scale;
        let grid_gap = snap(ShellMetrics::CARD_GAP * scale);
        let list_gap = snap(self.list_gap * scale);
        let card_height = snap(ShellMetrics::CARD_HEIGHT * scale);
        let height =
            |rows: usize, gap: f32| rows as f32 * card_height + rows.saturating_sub(1) as f32 * gap;
        let grid_height = height(grid_rows, grid_gap);
        let list_height = height(count, list_gap);

        div()
            .id(self.id)
            .test_support()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .h(px(grid_height + (list_height - grid_height) * progress))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let width = f32::from(bounds.size.width);
                        let track = (width - (columns - 1) as f32 * grid_gap) / columns as f32;
                        self.items
                            .into_iter()
                            .enumerate()
                            .map(|(index, (id, child))| {
                                let column = index % columns;
                                let row = index / columns;
                                let grid_left = snap(column as f32 * (track + grid_gap));
                                let grid_right =
                                    snap((column + 1) as f32 * (track + grid_gap)) - grid_gap;
                                let left = grid_left * (1.0 - progress);
                                let right = grid_right + (width - grid_right) * progress;
                                let grid_top = row as f32 * (card_height + grid_gap);
                                let list_top = index as f32 * (card_height + list_gap);
                                let top = grid_top + (list_top - grid_top) * progress;
                                let mut item = div()
                                    .id(id)
                                    .test_support()
                                    .w(px(right - left))
                                    .h(px(card_height))
                                    .min_w_0()
                                    .child(child)
                                    .into_any_element();
                                item.prepaint_as_root(
                                    bounds.origin + point(px(left), px(top)),
                                    size(
                                        AvailableSpace::Definite(px(right - left)),
                                        AvailableSpace::Definite(px(card_height)),
                                    ),
                                    window,
                                    cx,
                                );
                                item
                            })
                            .collect::<Vec<_>>()
                    },
                    |_, mut items, window, cx| {
                        for item in &mut items {
                            item.paint(window, cx);
                        }
                    },
                )
                .size_full(),
            )
    }
}
