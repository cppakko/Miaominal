use super::super::super::*;
use crate::ui::i18n;
use crate::ui::shell::pages::shell_compact_empty_state;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_component::{
    ElementExt,
    breadcrumb::{Breadcrumb, BreadcrumbItem},
    progress::Progress,
    table::DataTable,
};

const SFTP_SPLIT_GAP: f32 = 6.0;
const SFTP_ACTION_BUTTON_GAP: f32 = 4.0;
const SFTP_MIN_SPLIT_FLEX: f32 = 0.05;
const SFTP_DEFAULT_LOCAL_PANEL_FLEX: f32 = 0.5;
const SFTP_DEFAULT_BROWSER_AREA_FLEX: f32 = 0.95;
const SFTP_DEFAULT_BROWSER_AREA_FLEX_WITH_TRANSFERS: f32 = 0.76;
const SFTP_LOCAL_PANEL_MIN_WIDTH: f32 = 260.0;
const SFTP_REMOTE_PANEL_MIN_WIDTH: f32 = 260.0;
const SFTP_PROGRESS_CENTER_MIN_HEIGHT: f32 = 220.0;
const SFTP_BROWSER_MIN_HEIGHT: f32 = 240.0;
const SFTP_PROGRESS_CENTER_SLIDE_OFFSET: f32 = 14.0;
const SFTP_BREADCRUMB_MAX_VISIBLE_ITEMS: usize = 5;
const SFTP_BREADCRUMB_TRAILING_ITEMS: usize = 3;
const SFTP_BREADCRUMB_LABEL_MAX_CHARS: usize = 18;
const SFTP_BREADCRUMB_CURRENT_LABEL_MAX_CHARS: usize = 24;
const SFTP_BREADCRUMB_LABEL_MAX_WIDTH: f32 = 128.0;
const SFTP_BREADCRUMB_CURRENT_LABEL_MAX_WIDTH: f32 = 172.0;

#[derive(Clone)]
struct SftpSplitDragMarker {
    divider: SftpSplitDivider,
}

impl Render for SftpSplitDragMarker {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.divider;
        div().size(px(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_sftp_breadcrumb_indexes_keeps_short_paths() {
        assert_eq!(
            visible_sftp_breadcrumb_indexes(5),
            vec![Some(0), Some(1), Some(2), Some(3), Some(4)]
        );
    }

    #[test]
    fn visible_sftp_breadcrumb_indexes_collapses_middle_segments() {
        assert_eq!(
            visible_sftp_breadcrumb_indexes(7),
            vec![Some(0), None, Some(4), Some(5), Some(6)]
        );
    }

    #[test]
    fn sftp_breadcrumb_display_label_uses_smaller_limit_for_parent_segments() {
        assert_eq!(
            sftp_breadcrumb_display_label("abcdefghijklmnopqrstuv", false).as_ref(),
            "abcdefghijklmno..."
        );
        assert_eq!(
            sftp_breadcrumb_display_label("abcdefghijklmnopqrstuvwxyz", true).as_ref(),
            "abcdefghijklmnopqrstu..."
        );
    }

    #[test]
    fn transfer_order_stably_prioritizes_current_tab() {
        let mut transfers = vec![
            (TabId::new(2), "first"),
            (TabId::new(1), "current-a"),
            (TabId::new(2), "second"),
            (TabId::new(1), "current-b"),
        ];

        prioritize_sftp_transfer_tab(&mut transfers, Some(TabId::new(1)));

        assert_eq!(
            transfers,
            vec![
                (TabId::new(1), "current-a"),
                (TabId::new(1), "current-b"),
                (TabId::new(2), "first"),
                (TabId::new(2), "second")
            ]
        );
    }

    #[test]
    fn transfer_order_is_unchanged_without_a_current_tab() {
        let mut transfers = vec![(TabId::new(2), "first"), (TabId::new(1), "second")];

        prioritize_sftp_transfer_tab(&mut transfers, None);

        assert_eq!(
            transfers,
            vec![(TabId::new(2), "first"), (TabId::new(1), "second")]
        );
    }

    #[test]
    fn transfer_progress_metrics_cover_known_and_unknown_totals() {
        assert_eq!(
            sftp_progress_metrics(25, Some(100), false, true),
            SftpProgressMetrics {
                value: 25.0,
                loading: false,
                percent: Some(25),
            }
        );
        assert_eq!(
            sftp_progress_metrics(32, None, false, true),
            SftpProgressMetrics {
                value: 0.0,
                loading: true,
                percent: None,
            }
        );
        assert_eq!(
            sftp_progress_metrics(32, None, true, false),
            SftpProgressMetrics {
                value: 100.0,
                loading: false,
                percent: Some(100),
            }
        );
        assert_eq!(
            sftp_progress_metrics(0, Some(0), true, false),
            SftpProgressMetrics {
                value: 100.0,
                loading: false,
                percent: Some(100),
            }
        );
    }

    #[test]
    fn transfer_status_tones_keep_failure_distinct_from_warning() {
        assert_eq!(
            sftp_transfer_status_tone(&SftpTransferStatus::Queued),
            SftpTransferTone::Neutral
        );
        assert_eq!(
            sftp_transfer_status_tone(&SftpTransferStatus::Paused),
            SftpTransferTone::Warning
        );
        assert_eq!(
            sftp_transfer_status_tone(&SftpTransferStatus::Failed("boom".into())),
            SftpTransferTone::Error
        );
        assert_eq!(
            sftp_transfer_child_status_tone(&SftpTransferChildStatus::Failed("boom".into())),
            SftpTransferTone::Error
        );
    }

    #[test]
    fn transfer_action_ids_are_unique_by_tab_transfer_and_action() {
        let pause = sftp_transfer_action_id(TabId::new(1), TransferId(7), "pause");
        let resume = sftp_transfer_action_id(TabId::new(1), TransferId(7), "resume");
        let other_tab = sftp_transfer_action_id(TabId::new(2), TransferId(7), "pause");
        let other_transfer = sftp_transfer_action_id(TabId::new(1), TransferId(8), "pause");

        assert_ne!(pause, resume);
        assert_ne!(pause, other_tab);
        assert_ne!(pause, other_transfer);
    }

    #[test]
    fn transfer_display_name_prefers_source_basename() {
        let transfer = SftpTransferRow {
            transfer_id: TransferId(1),
            direction: TransferDirection::Upload,
            source: PathBuf::from(r"C:\work\archive.zip"),
            destination: "/srv/archive.zip".into(),
            bytes_complete: 0,
            bytes_total: None,
            status: SftpTransferStatus::Queued,
            bytes_per_second: None,
            last_progress_at: None,
            last_bytes_complete: 0,
            is_directory: false,
            expanded: false,
            children: std::collections::VecDeque::new(),
            child_count: 0,
        };

        assert_eq!(sftp_transfer_display_name(&transfer), "archive.zip");
    }
}

fn sftp_usable_container_size(size: Pixels) -> f32 {
    (size.as_f32() - SFTP_SPLIT_GAP).max(1.0)
}

fn clamp_sftp_local_panel_flex(container_width: Pixels, requested: f32) -> f32 {
    let available = sftp_usable_container_size(container_width);
    let min = (SFTP_LOCAL_PANEL_MIN_WIDTH / available).clamp(SFTP_MIN_SPLIT_FLEX, 0.95);
    let max = (1.0 - (SFTP_REMOTE_PANEL_MIN_WIDTH / available).clamp(SFTP_MIN_SPLIT_FLEX, 0.95))
        .clamp(0.05, 0.95);

    if max <= min {
        return 0.5;
    }

    requested.clamp(min, max)
}

fn default_sftp_browser_area_flex(tab: &SftpTabState) -> f32 {
    if tab.transfers.is_empty() {
        SFTP_DEFAULT_BROWSER_AREA_FLEX
    } else {
        SFTP_DEFAULT_BROWSER_AREA_FLEX_WITH_TRANSFERS
    }
}

fn clamp_sftp_browser_area_flex(container_height: Pixels, requested: f32) -> f32 {
    let available = sftp_usable_container_size(container_height);
    let min = (SFTP_BROWSER_MIN_HEIGHT / available).clamp(SFTP_MIN_SPLIT_FLEX, 0.95);
    let max = (1.0
        - (SFTP_PROGRESS_CENTER_MIN_HEIGHT / available).clamp(SFTP_MIN_SPLIT_FLEX, 0.95))
    .clamp(0.05, 0.95);

    if max <= min {
        return 0.5;
    }

    requested.clamp(min, max)
}

fn sftp_local_panel_flex(tab: &SftpTabState) -> f32 {
    clamp_sftp_local_panel_flex(
        tab.layout.browser_container_width,
        tab.layout
            .local_panel_flex
            .unwrap_or(SFTP_DEFAULT_LOCAL_PANEL_FLEX),
    )
}

fn sftp_browser_area_flex(tab: &SftpTabState) -> f32 {
    clamp_sftp_browser_area_flex(
        tab.layout.page_container_height,
        tab.layout
            .browser_area_flex
            .unwrap_or_else(|| default_sftp_browser_area_flex(tab)),
    )
}

fn sftp_drag_selection_overlay_bounds(
    drag: SftpDragSelectionState,
    header_height: Pixels,
) -> Bounds<Pixels> {
    let bounds = drag.bounds();
    let left = bounds.origin.x;
    let right = bounds.origin.x + bounds.size.width;
    let top = bounds.origin.y.max(header_height);
    let bottom = (bounds.origin.y + bounds.size.height).max(header_height);

    Bounds::from_corners(Point::new(left, top), Point::new(right, bottom))
}

fn sftp_path_input_shell(input: &Entity<InputState>) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;

    div()
        .flex_1()
        .min_w(px(0.0))
        .h(px(30.0))
        .rounded(px(8.0))
        .bg(rgb(roles.surface_container))
        .border_1()
        .border_color(rgb(roles.outline_variant))
        .px_3()
        .flex()
        .items_center()
        .overflow_hidden()
        .child(
            HintedInput::new(input)
                .appearance(false)
                .border_0()
                .small()
                .h_full()
                .w_full(),
        )
}

fn sftp_path_button(
    icon: AppIcon,
    tooltip: impl Into<SharedString>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;

    icon_button_with_tooltip(
        icon,
        tooltip,
        28.0,
        8.0,
        Some(roles.surface_container_low),
        Some(roles.on_surface_variant),
        Some(roles.outline_variant),
        on_click,
    )
    .flex_shrink_0()
}

fn sftp_path_tooltip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView {
    let text = text.into();

    move |window, cx| gpui_component::tooltip::Tooltip::new(text.clone()).build(window, cx)
}

fn sftp_path_breadcrumb_shell(
    id: impl Into<ElementId>,
    content: impl IntoElement,
    full_path: impl Into<SharedString>,
) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let full_path = full_path.into();

    div()
        .id(id)
        .flex_1()
        .min_w(px(0.0))
        .h(px(30.0))
        .rounded(px(99.0))
        .bg(rgb(roles.surface_container))
        .px_3()
        .flex()
        .items_center()
        .overflow_hidden()
        .tooltip(sftp_path_tooltip(full_path))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(content),
        )
}

fn visible_sftp_breadcrumb_indexes(items_len: usize) -> Vec<Option<usize>> {
    if items_len <= SFTP_BREADCRUMB_MAX_VISIBLE_ITEMS {
        return (0..items_len).map(Some).collect();
    }

    let trailing_count = SFTP_BREADCRUMB_TRAILING_ITEMS.min(items_len.saturating_sub(1));
    let trailing_start = items_len - trailing_count;
    let mut indexes = Vec::with_capacity(2 + trailing_count);
    indexes.push(Some(0));
    indexes.push(None);
    indexes.extend((trailing_start..items_len).map(Some));
    indexes
}

fn sftp_breadcrumb_display_label(label: &str, is_current: bool) -> SharedString {
    let max_chars = if is_current {
        SFTP_BREADCRUMB_CURRENT_LABEL_MAX_CHARS
    } else {
        SFTP_BREADCRUMB_LABEL_MAX_CHARS
    };

    truncate_with_ellipsis(label, max_chars).into()
}

fn sftp_breadcrumb_item(label: SharedString, is_current: bool) -> BreadcrumbItem {
    let max_width = if is_current {
        SFTP_BREADCRUMB_CURRENT_LABEL_MAX_WIDTH
    } else {
        SFTP_BREADCRUMB_LABEL_MAX_WIDTH
    };

    BreadcrumbItem::new(label)
        .min_w(px(0.0))
        .max_w(px(max_width))
        .flex_shrink(1.0)
        .truncate()
}

fn local_sftp_breadcrumb_label(path: &Path) -> SharedString {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned().into())
        .unwrap_or_else(|| SftpController::display_local_path(path))
}

fn build_local_sftp_breadcrumb(
    path: &Path,
    controller: Entity<SftpController>,
    tab_id: TabId,
) -> Breadcrumb {
    let mut breadcrumb = Breadcrumb::new().w_full().min_w(px(0.0)).overflow_hidden();
    let mut ancestors: Vec<PathBuf> = path
        .ancestors()
        .map(|ancestor| ancestor.to_path_buf())
        .collect();
    ancestors.reverse();
    let visible_indexes = visible_sftp_breadcrumb_indexes(ancestors.len());

    for visible_index in visible_indexes {
        let Some(index) = visible_index else {
            breadcrumb = breadcrumb.child(
                BreadcrumbItem::new("...")
                    .disabled(true)
                    .flex_shrink_0()
                    .truncate(),
            );
            continue;
        };

        let Some(ancestor) = ancestors.get(index) else {
            continue;
        };

        let raw_label = local_sftp_breadcrumb_label(ancestor);
        let is_current = ancestor.as_path() == path;
        let label = sftp_breadcrumb_display_label(raw_label.as_ref(), is_current);
        let item = sftp_breadcrumb_item(label, is_current);
        let item = if is_current {
            item.disabled(true)
        } else {
            let click_controller = controller.clone();
            let target = ancestor.clone();
            item.on_click(move |_, _, cx| {
                let target = target.clone();
                click_controller.update(cx, |controller, cx| {
                    controller.navigate_local_to_path(tab_id, target.clone(), cx);
                });
            })
        };
        breadcrumb = breadcrumb.child(item);
    }

    breadcrumb
}

fn build_remote_sftp_breadcrumb(
    path: &str,
    controller: Entity<SftpController>,
    tab_id: TabId,
) -> Breadcrumb {
    let trimmed = path.trim();
    let current_path = if trimmed.is_empty() { "." } else { trimmed };
    let mut segments: Vec<(String, SharedString)> = Vec::new();

    if current_path == "/" {
        segments.push(("/".into(), "/".into()));
    } else if current_path == "." {
        segments.push((".".into(), ".".into()));
    } else if current_path.starts_with('/') {
        segments.push(("/".into(), "/".into()));
        let mut accumulated = "/".to_string();
        for segment in current_path
            .trim_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
        {
            accumulated = SftpController::join_remote_path(&accumulated, segment);
            segments.push((accumulated.clone(), segment.to_string().into()));
        }
    } else {
        let mut accumulated = ".".to_string();
        segments.push((accumulated.clone(), ".".into()));
        for segment in current_path
            .split('/')
            .filter(|segment| !segment.is_empty() && *segment != ".")
        {
            accumulated = SftpController::join_remote_path(&accumulated, segment);
            segments.push((accumulated.clone(), segment.to_string().into()));
        }
    }

    let mut breadcrumb = Breadcrumb::new().w_full().min_w(px(0.0)).overflow_hidden();
    let visible_indexes = visible_sftp_breadcrumb_indexes(segments.len());

    for visible_index in visible_indexes {
        let Some(index) = visible_index else {
            breadcrumb = breadcrumb.child(
                BreadcrumbItem::new("...")
                    .disabled(true)
                    .flex_shrink_0()
                    .truncate(),
            );
            continue;
        };

        let Some((target_path, raw_label)) = segments.get(index) else {
            continue;
        };

        let is_current = target_path == current_path;
        let label = sftp_breadcrumb_display_label(raw_label.as_ref(), is_current);
        let item = sftp_breadcrumb_item(label, is_current);
        let item = if is_current {
            item.disabled(true)
        } else {
            let click_controller = controller.clone();
            let target = target_path.clone();
            item.on_click(move |_, _, cx| {
                let target = target.clone();
                click_controller.update(cx, |controller, cx| {
                    controller.request_remote_directory(tab_id, target.clone(), cx);
                });
            })
        };
        breadcrumb = breadcrumb.child(item);
    }

    breadcrumb
}

fn sftp_path_bar(
    path_content: impl IntoElement,
    show_edit_button: bool,
    on_up: impl Fn(&mut Window, &mut App) + 'static,
    on_edit: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .min_w(px(0.0))
        .items_center()
        .gap(px(SFTP_ACTION_BUTTON_GAP))
        .child(path_content)
        .child(sftp_path_button(
            AppIcon::CornerLeftUp,
            i18n::string("sftp.tooltips.go_up"),
            move |window, cx| on_up(window, cx),
        ))
        .when(show_edit_button, |this| {
            this.child(sftp_path_button(
                AppIcon::Edit,
                i18n::string("sftp.tooltips.edit_path"),
                move |window, cx| on_edit(window, cx),
            ))
        })
}

fn sftp_panel_card() -> Div {
    let roles = miaominal_settings::current_theme().material.roles;

    card_surface(roles.surface_container_highest, 8.0)
        .size_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
}

fn sftp_toolbar_button(
    icon: AppIcon,
    tooltip: impl Into<SharedString>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;

    icon_button_with_tooltip(
        icon,
        tooltip,
        28.0,
        8.0,
        Some(roles.surface_container_low),
        Some(roles.on_surface_variant),
        Some(roles.outline_variant),
        on_click,
    )
}

fn sftp_panel_meta_label(item_count: usize, selected_count: usize) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let label = if selected_count == 0 {
        i18n::string_args("sftp.ui.item_count", &[("count", &item_count.to_string())])
    } else {
        i18n::string_args(
            "sftp.ui.selection_count",
            &[
                ("selected", &selected_count.to_string()),
                ("count", &item_count.to_string()),
            ],
        )
    };

    div()
        .flex_shrink_0()
        .text_size(miaominal_settings::FontSize::Body.scaled())
        .line_height(miaominal_settings::scaled_line_height(16.0))
        .text_color(rgb(if selected_count == 0 {
            roles.on_surface_variant
        } else {
            roles.primary
        }))
        .child(label)
}

fn sftp_split_bar(
    controller: Entity<SftpController>,
    tab_id: TabId,
    divider: SftpSplitDivider,
    is_dragging: bool,
) -> gpui::AnyElement {
    let bar_id = SharedString::from(format!(
        "sftp-split-bar-{tab_id}-{}",
        match divider {
            SftpSplitDivider::BrowserPanels => "browser",
            SftpSplitDivider::ProgressCenter => "progress",
        }
    ));

    let roles = miaominal_settings::current_theme().material.roles;
    let marker = SftpSplitDragMarker { divider };
    let mut bar = div().id(bar_id).flex_shrink_0().occlude();
    bar = match divider {
        SftpSplitDivider::BrowserPanels => bar.w(px(SFTP_SPLIT_GAP)).h_full().cursor_col_resize(),
        SftpSplitDivider::ProgressCenter => bar.h(px(SFTP_SPLIT_GAP)).w_full().cursor_row_resize(),
    };

    bar.on_mouse_down(
        MouseButton::Left,
        move |event: &MouseDownEvent, _window, cx| {
            let pointer = match divider {
                SftpSplitDivider::BrowserPanels => f32::from(event.position.x),
                SftpSplitDivider::ProgressCenter => f32::from(event.position.y),
            };
            controller.update(cx, |controller, cx| {
                controller.start_sftp_split_drag(tab_id, divider, pointer, cx);
            });
        },
    )
    .hover(move |this| {
        if is_dragging {
            this.bg(color_with_alpha(roles.primary, 0x22))
        } else {
            match divider {
                SftpSplitDivider::BrowserPanels => this
                    .cursor_col_resize()
                    .bg(color_with_alpha(roles.primary, 0x14)),
                SftpSplitDivider::ProgressCenter => this
                    .cursor_row_resize()
                    .bg(color_with_alpha(roles.primary, 0x14)),
            }
        }
    })
    .on_drag(marker, |m, _offset, _window, cx| cx.new(|_| m.clone()))
    .into_any_element()
}

struct SftpBrowserSection<P, T, C, M> {
    section_id: ElementId,
    title: SharedString,
    show_title: bool,
    icon: AppIcon,
    item_count: usize,
    selected_count: usize,
    path_bar: P,
    toolbar: T,
    content: C,
    menu_builder: M,
}

fn sftp_browser_section<P, T, C, M>(section: SftpBrowserSection<P, T, C, M>) -> impl IntoElement
where
    P: IntoElement,
    T: IntoElement,
    C: IntoElement,
    M: for<'a, 'b, 'c> Fn(PopupMenu, &'a mut Window, &'b mut Context<'c, PopupMenu>) -> PopupMenu
        + 'static,
{
    let roles = miaominal_settings::current_theme().material.roles;
    let SftpBrowserSection {
        section_id,
        title,
        show_title,
        icon,
        item_count,
        selected_count,
        path_bar,
        toolbar,
        content,
        menu_builder,
    } = section;

    sftp_panel_card()
        .id(section_id)
        .flex()
        .flex_col()
        .context_menu(menu_builder)
        .child(
            div().w_full().flex_shrink_0().px_3().pt_2().pb_2().child(
                v_flex()
                    .w_full()
                    .flex_shrink_0()
                    .gap_2()
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(24.0))
                            .flex_shrink_0()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .size(px(18.0))
                                            .flex_shrink_0()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_color(rgb(roles.on_surface_variant))
                                            .child(Icon::new(icon).size(px(16.0))),
                                    )
                                    .when(show_title, |this| {
                                        this.child(
                                            div()
                                                .min_w(px(0.0))
                                                .flex_shrink_0()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_size(
                                                    miaominal_settings::FontSize::Input.scaled(),
                                                )
                                                .line_height(
                                                    miaominal_settings::scaled_line_height(20.0),
                                                )
                                                .text_color(rgb(roles.on_surface))
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(title),
                                        )
                                    }),
                            )
                            .child(sftp_panel_meta_label(item_count, selected_count)),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .min_w(px(0.0))
                            .flex_shrink_0()
                            .items_center()
                            .gap(px(SFTP_ACTION_BUTTON_GAP))
                            .child(path_bar)
                            .child(toolbar),
                    ),
            ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(content),
        )
}

const SFTP_TRANSFER_ACTION_SIZE: f32 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SftpTransferTone {
    Neutral,
    Info,
    Warning,
    Success,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SftpProgressMetrics {
    value: f32,
    loading: bool,
    percent: Option<u8>,
}

fn sftp_progress_metrics(
    bytes_complete: u64,
    bytes_total: Option<u64>,
    done: bool,
    loading: bool,
) -> SftpProgressMetrics {
    match bytes_total {
        Some(total) if total > 0 => {
            let value = ((bytes_complete as f32 / total as f32) * 100.0).clamp(0.0, 100.0);
            SftpProgressMetrics {
                value,
                loading: false,
                percent: Some(value.round() as u8),
            }
        }
        Some(_) => SftpProgressMetrics {
            value: if done { 100.0 } else { 0.0 },
            loading: false,
            percent: Some(if done { 100 } else { 0 }),
        },
        None if done => SftpProgressMetrics {
            value: 100.0,
            loading: false,
            percent: Some(100),
        },
        None => SftpProgressMetrics {
            value: 0.0,
            loading,
            percent: None,
        },
    }
}

fn sftp_transfer_status_tone(status: &SftpTransferStatus) -> SftpTransferTone {
    match status {
        SftpTransferStatus::Queued | SftpTransferStatus::Cancelled => SftpTransferTone::Neutral,
        SftpTransferStatus::Running => SftpTransferTone::Info,
        SftpTransferStatus::Paused => SftpTransferTone::Warning,
        SftpTransferStatus::Done => SftpTransferTone::Success,
        SftpTransferStatus::Failed(_) => SftpTransferTone::Error,
    }
}

fn sftp_transfer_child_status_tone(status: &SftpTransferChildStatus) -> SftpTransferTone {
    match status {
        SftpTransferChildStatus::Running => SftpTransferTone::Info,
        SftpTransferChildStatus::Paused => SftpTransferTone::Warning,
        SftpTransferChildStatus::Done => SftpTransferTone::Success,
        SftpTransferChildStatus::Cancelled => SftpTransferTone::Neutral,
        SftpTransferChildStatus::Failed(_) => SftpTransferTone::Error,
    }
}

fn sftp_transfer_tone_colors(tone: SftpTransferTone) -> (u32, u32, u32) {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let extended = material.extended;

    match tone {
        SftpTransferTone::Neutral => (
            roles.on_surface_variant,
            roles.surface_container_highest,
            roles.on_surface_variant,
        ),
        SftpTransferTone::Info => (
            extended.info.color,
            extended.info.color_container,
            extended.info.on_color_container,
        ),
        SftpTransferTone::Warning => (
            extended.warning.color,
            extended.warning.color_container,
            extended.warning.on_color_container,
        ),
        SftpTransferTone::Success => (
            extended.success.color,
            extended.success.color_container,
            extended.success.on_color_container,
        ),
        SftpTransferTone::Error => (roles.error, roles.error_container, roles.on_error_container),
    }
}

fn sftp_transfer_status_label(status: &SftpTransferStatus) -> String {
    i18n::string(match status {
        SftpTransferStatus::Queued => "sftp.transfer_status.queued",
        SftpTransferStatus::Running => "sftp.transfer_status.running",
        SftpTransferStatus::Paused => "sftp.transfer_status.paused",
        SftpTransferStatus::Done => "sftp.transfer_status.done",
        SftpTransferStatus::Cancelled => "sftp.transfer_status.cancelled",
        SftpTransferStatus::Failed(_) => "sftp.transfer_status.failed_short",
    })
}

fn sftp_transfer_child_status_label(status: &SftpTransferChildStatus) -> String {
    i18n::string(match status {
        SftpTransferChildStatus::Running => "sftp.transfer_status.running",
        SftpTransferChildStatus::Paused => "sftp.transfer_status.paused",
        SftpTransferChildStatus::Done => "sftp.transfer_status.done",
        SftpTransferChildStatus::Cancelled => "sftp.transfer_status.cancelled",
        SftpTransferChildStatus::Failed(_) => "sftp.transfer_status.failed_short",
    })
}

fn sftp_transfer_display_name(transfer: &SftpTransferRow) -> String {
    transfer
        .source
        .file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .or_else(|| {
            Path::new(&transfer.destination)
                .file_name()
                .filter(|name| !name.is_empty())
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| transfer.source.display().to_string())
}

fn prioritize_sftp_transfer_tab<T>(items: &mut [(TabId, T)], preferred_tab_id: Option<TabId>) {
    if let Some(preferred_tab_id) = preferred_tab_id {
        items.sort_by_key(|(tab_id, _)| *tab_id != preferred_tab_id);
    }
}

fn sftp_transfer_action_id(tab_id: TabId, transfer_id: TransferId, action: &str) -> SharedString {
    SharedString::from(format!("sftp-transfer-{action}-{tab_id}-{}", transfer_id.0))
}

fn sftp_transfer_action_button(
    id: impl Into<ElementId>,
    icon: AppIcon,
    tooltip: impl Into<SharedString>,
    destructive: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    let roles = miaominal_settings::current_theme().material.roles;
    let background = if destructive {
        roles.error_container
    } else {
        roles.surface_container_low
    };
    let foreground = if destructive {
        roles.on_error_container
    } else {
        roles.on_surface_variant
    };

    let button = Button::new(id.into())
        .text()
        .tooltip(tooltip.into())
        .size(px(SFTP_TRANSFER_ACTION_SIZE))
        .p_0()
        .rounded(px(9.0))
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .child(Icon::new(icon).small())
        .on_click(move |_, window, cx| on_click(window, cx));

    div().size(px(SFTP_TRANSFER_ACTION_SIZE)).child(button)
}

fn sftp_progress_center_card(
    section_id: ElementId,
    header: impl IntoElement,
    content: impl IntoElement,
) -> impl IntoElement {
    let content_shell = div()
        .w_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .child(content);

    sftp_panel_card()
        .id(section_id)
        .flex()
        .flex_col()
        .child(div().w_full().flex_shrink_0().child(header))
        .child(content_shell)
}

fn sftp_empty_transfer_summary() -> impl IntoElement {
    shell_compact_empty_state(
        AppIcon::ArrowUpDown,
        i18n::string("sftp.ui.transfer_idle"),
        0.0,
    )
}

fn context_menu_local_sftp_entry(
    controller: &Entity<SftpController>,
    tab_id: TabId,
    cx: &App,
) -> Option<LocalSftpEntry> {
    let row_ix = controller
        .read(cx)
        .local_table()
        .read(cx)
        .right_clicked_row()?;
    let path = controller
        .read(cx)
        .local_table()
        .read(cx)
        .delegate()
        .row(row_ix)
        .map(|row| PathBuf::from(row.path.as_str()))?;

    controller.read(cx).tab(tab_id).and_then(|tab| {
        tab.local_entries
            .iter()
            .find(|entry| entry.path == path)
            .cloned()
    })
}

fn context_menu_remote_sftp_entry(
    controller: &Entity<SftpController>,
    tab_id: TabId,
    cx: &App,
) -> Option<SftpEntry> {
    let remote_table = controller.read(cx).remote_table();
    let table = remote_table.read(cx);
    let row_ix = table.right_clicked_row()?;
    let path = table.delegate().row(row_ix).map(|row| row.path.clone())?;
    controller.read(cx).resolve_remote_entry(tab_id, &path, cx)
}

fn build_local_sftp_context_menu(
    menu: PopupMenu,
    controller: Entity<SftpController>,
    tab_id: TabId,
    cx: &App,
) -> PopupMenu {
    let mut menu = menu;

    if let Some(entry) = context_menu_local_sftp_entry(&controller, tab_id, cx) {
        if entry.is_directory {
            let open_controller = controller.clone();
            let open_path = entry.path.clone();
            menu = menu.item(PopupMenuItem::new(i18n::string("sftp.menu.open")).on_click(
                move |_, _, cx| {
                    let controller = open_controller.clone();
                    let path = open_path.clone();
                    controller.update(cx, |controller, cx| {
                        controller.select_local_path(tab_id, path.clone(), cx);
                        controller.navigate_local_into_selected(tab_id, cx);
                    });
                },
            ));
        }

        let upload_controller = controller.clone();
        let upload_path = entry.path.clone();
        menu = menu
            .item(
                PopupMenuItem::new(i18n::string("sftp.menu.upload")).on_click(move |_, _, cx| {
                    let controller = upload_controller.clone();
                    let path = upload_path.clone();
                    let already_selected = controller
                        .read(cx)
                        .tab(tab_id)
                        .is_some_and(|tab| tab.selected_local_paths.iter().any(|p| p == &path));
                    if !already_selected {
                        controller.update(cx, |controller, cx| {
                            controller.select_local_path(tab_id, path.clone(), cx);
                        });
                    }
                    controller.update(cx, |controller, cx| {
                        controller.queue_upload_selected(tab_id, cx);
                    });
                }),
            )
            .item(PopupMenuItem::separator());
    }

    let up_controller = controller;
    let refresh_controller = up_controller.clone();
    menu.item(
        PopupMenuItem::new(i18n::string("sftp.menu.go_up")).on_click(move |_, _, cx| {
            let controller = up_controller.clone();
            controller.update(cx, |controller, cx| {
                controller.navigate_local_up(tab_id, cx);
            });
        }),
    )
    .item(
        PopupMenuItem::new(i18n::string("sftp.menu.refresh")).on_click(move |_, _, cx| {
            let controller = refresh_controller.clone();
            controller.update(cx, |controller, cx| {
                controller.refresh_local_directory(tab_id, cx);
            });
        }),
    )
}

fn build_remote_sftp_context_menu(
    menu: PopupMenu,
    controller: Entity<SftpController>,
    tab_id: TabId,
    prompt_for_destination: bool,
    cx: &App,
) -> PopupMenu {
    let mut menu = menu;

    if let Some(entry) = context_menu_remote_sftp_entry(&controller, tab_id, cx) {
        let is_single_selection = controller
            .read(cx)
            .tab(tab_id)
            .map(|tab| tab.selected_remote_paths.len() == 1)
            .unwrap_or(false);

        if entry.kind == miaominal_sftp::SftpEntryKind::Directory {
            let open_controller = controller.clone();
            let open_path = entry.path.clone();
            menu = menu.item(PopupMenuItem::new(i18n::string("sftp.menu.open")).on_click(
                move |_, _, cx| {
                    let controller = open_controller.clone();
                    let path = open_path.clone();
                    controller.update(cx, |controller, cx| {
                        controller.select_remote_path(tab_id, path.clone(), cx);
                        controller.navigate_remote_into_selected(tab_id, cx);
                    });
                },
            ));
        }

        let download_controller = controller.clone();
        let download_path = entry.path.clone();
        let edit_controller = controller.clone();
        let edit_path = entry.path.clone();
        let is_file = entry.kind != miaominal_sftp::SftpEntryKind::Directory;
        let rename_controller = controller.clone();
        let delete_controller = controller.clone();
        menu = menu.item(
            PopupMenuItem::new(i18n::string("sftp.menu.download")).on_click(
                move |_, window, cx| {
                    let controller = download_controller.clone();
                    let path = download_path.clone();
                    let already_selected = controller
                        .read(cx)
                        .tab(tab_id)
                        .is_some_and(|tab| tab.selected_remote_paths.iter().any(|p| p == &path));
                    if !already_selected {
                        controller.update(cx, |controller, cx| {
                            controller.select_remote_path(tab_id, path.clone(), cx);
                        });
                    }
                    controller.update(cx, |controller, cx| {
                        controller.queue_download_selected(
                            tab_id,
                            prompt_for_destination,
                            window,
                            cx,
                        );
                    });
                },
            ),
        );
        if is_single_selection && is_file {
            menu = menu.item(PopupMenuItem::new(i18n::string("sftp.menu.edit")).on_click(
                move |_, _, cx| {
                    let controller = edit_controller.clone();
                    let path = edit_path.clone();
                    controller.update(cx, |controller, cx| {
                        controller.open_remote_file_for_editing(tab_id, path, cx);
                    });
                },
            ));
        }
        if is_single_selection {
            menu = menu.item(
                PopupMenuItem::new(i18n::string("sftp.menu.rename")).on_click(
                    move |_, window, cx| {
                        let controller = rename_controller.clone();
                        controller.update(cx, |controller, cx| {
                            controller.begin_inline_rename(tab_id, window, cx);
                        });
                    },
                ),
            );
        }
        menu = menu
            .item(
                PopupMenuItem::new(i18n::string("sftp.menu.delete")).on_click(move |_, _, cx| {
                    let controller = delete_controller.clone();
                    controller.update(cx, |controller, cx| {
                        controller.delete_remote_selected(tab_id, cx);
                    });
                }),
            )
            .item(PopupMenuItem::separator());
    }

    let up_controller = controller;
    let refresh_controller = up_controller.clone();
    let create_controller = up_controller.clone();
    menu.item(
        PopupMenuItem::new(i18n::string("sftp.menu.go_up")).on_click(move |_, _, cx| {
            let controller = up_controller.clone();
            controller.update(cx, |controller, cx| {
                controller.navigate_remote_up(tab_id, cx);
            });
        }),
    )
    .item(
        PopupMenuItem::new(i18n::string("sftp.menu.refresh")).on_click(move |_, _, cx| {
            let controller = refresh_controller.clone();
            let path = controller
                .read(cx)
                .tab(tab_id)
                .map(|tab| tab.remote_path.clone())
                .unwrap_or_else(|| ".".into());
            controller.update(cx, |controller, cx| {
                controller.request_remote_directory(tab_id, path, cx);
            });
        }),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("sftp.menu.create_directory")).on_click(
            move |_, window, cx| {
                let controller = create_controller.clone();
                controller.update(cx, |controller, cx| {
                    controller.begin_create_directory(tab_id, window, cx);
                });
            },
        ),
    )
}

impl SftpController {
    pub(in crate::ui::shell) fn render_sftp_progress_center(
        &self,
        controller: Entity<Self>,
        section_id: impl Into<ElementId>,
        ordered_tab_ids: &[TabId],
        preferred_tab_id: Option<TabId>,
    ) -> gpui::AnyElement {
        let section_id = section_id.into();
        let sftp_tabs = self.tabs();
        let mut transfers: Vec<(TabId, (&SftpTabState, &SftpTransferRow))> = self
            .ordered_sftp_tab_ids(ordered_tab_ids)
            .filter_map(|tab_id| sftp_tabs.get(&tab_id).map(|sftp| (tab_id, sftp)))
            .flat_map(|(tab_id, sftp)| {
                sftp.transfers
                    .iter()
                    .map(move |transfer| (tab_id, (sftp, transfer)))
            })
            .collect();
        prioritize_sftp_transfer_tab(&mut transfers, preferred_tab_id);

        let transfer_count = transfers.len();
        let active_count = transfers
            .iter()
            .filter(|(_, (_, transfer))| {
                matches!(
                    transfer.status,
                    SftpTransferStatus::Queued
                        | SftpTransferStatus::Running
                        | SftpTransferStatus::Paused
                )
            })
            .count();
        let failed_count = transfers
            .iter()
            .filter(|(_, (_, transfer))| matches!(transfer.status, SftpTransferStatus::Failed(_)))
            .count();
        let header =
            self.render_sftp_progress_center_header(transfer_count, active_count, failed_count);
        let scroll_handle = self.progress_scroll_handle();

        let content = if transfers.is_empty() {
            div()
                .size_full()
                .min_h(px(0.0))
                .child(sftp_empty_transfer_summary())
                .into_any_element()
        } else {
            let mut rows = v_flex().w_full().gap_2().flex_shrink_0();
            let profiles = self.session_profiles();
            for (tab_id, (sftp_tab, transfer)) in transfers {
                rows = rows.child(self.render_sftp_transfer_card(
                    controller.clone(),
                    tab_id,
                    sftp_tab,
                    transfer,
                    &profiles,
                ));
            }

            div()
                .id("sftp-progress-center-scroll-shell")
                .relative()
                .size_full()
                .min_h(px(0.0))
                .child(
                    div()
                        .id("sftp-progress-center-scroll")
                        .size_full()
                        .track_scroll(&scroll_handle)
                        .overflow_y_scroll()
                        .p_2()
                        .child(rows),
                )
                .vertical_scrollbar(&scroll_handle)
                .into_any_element()
        };

        sftp_progress_center_card(section_id, header, content).into_any_element()
    }

    fn ordered_sftp_tab_ids<'a>(
        &'a self,
        ordered_tab_ids: &'a [TabId],
    ) -> impl Iterator<Item = TabId> + 'a {
        ordered_tab_ids.iter().copied()
    }

    fn render_sftp_progress_center_header(
        &self,
        transfer_count: usize,
        active_count: usize,
        failed_count: usize,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let extended = material.extended;

        h_flex()
            .w_full()
            .min_w(px(0.0))
            .h(px(46.0))
            .items_center()
            .gap_2()
            .px_3()
            .child(
                div()
                    .size(px(30.0))
                    .flex_shrink_0()
                    .rounded(px(9.0))
                    .bg(rgb(roles.surface_container_low))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(roles.primary))
                    .child(Icon::new(AppIcon::ArrowUpDown).small()),
            )
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Heading.scaled())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(roles.on_surface))
                    .child(i18n::string("sftp.ui.transfer_center_title")),
            )
            .child(badge(
                i18n::string_args(
                    "sftp.ui.transfer_count",
                    &[("count", &transfer_count.to_string())],
                ),
                roles.surface_container_highest,
                roles.on_surface_variant,
            ))
            .when(active_count > 0, |this| {
                this.child(badge(
                    i18n::string_args(
                        "sftp.ui.transfer_active_count",
                        &[("count", &active_count.to_string())],
                    ),
                    extended.info.color_container,
                    extended.info.on_color_container,
                ))
            })
            .when(failed_count > 0, |this| {
                this.child(badge(
                    i18n::string_args(
                        "sftp.ui.transfer_failed_count",
                        &[("count", &failed_count.to_string())],
                    ),
                    roles.error_container,
                    roles.on_error_container,
                ))
            })
            .into_any_element()
    }

    fn render_sftp_transfer_card(
        &self,
        controller: Entity<Self>,
        tab_id: TabId,
        sftp_tab: &SftpTabState,
        transfer: &SftpTransferRow,
        profiles: &[SessionProfile],
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let transfer_id = transfer.transfer_id;
        let profile_label = profiles
            .iter()
            .find(|profile| profile.id == sftp_tab.profile_id)
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| sftp_tab.profile_id.clone());
        let profile_display = truncate_with_ellipsis(&profile_label, 20);
        let display_name = sftp_transfer_display_name(transfer);
        let route = format!("{} → {}", transfer.source.display(), transfer.destination);
        let direction_icon = match transfer.direction {
            TransferDirection::Upload => AppIcon::Upload,
            TransferDirection::Download => AppIcon::Download,
        };
        let done = matches!(transfer.status, SftpTransferStatus::Done);
        let loading = matches!(
            transfer.status,
            SftpTransferStatus::Queued | SftpTransferStatus::Running
        );
        let progress_metrics =
            sftp_progress_metrics(transfer.bytes_complete, transfer.bytes_total, done, loading);
        let progress_label = transfer.bytes_total.map_or_else(
            || format_byte_size(Some(transfer.bytes_complete)).to_string(),
            |total| {
                format!(
                    "{} / {}",
                    format_byte_size(Some(transfer.bytes_complete)),
                    format_byte_size(Some(total))
                )
            },
        );
        let mut progress_details = vec![progress_label];
        if let Some(percent) = progress_metrics.percent {
            progress_details.push(format!("{percent}%"));
        }
        if matches!(transfer.status, SftpTransferStatus::Running)
            && let Some(bytes_per_second) = transfer.bytes_per_second
        {
            progress_details.push(format!("{}/s", format_byte_size(Some(bytes_per_second))));
        }
        let progress_details = progress_details.join(" · ");
        let tone = sftp_transfer_status_tone(&transfer.status);
        let (accent, badge_background, badge_foreground) = sftp_transfer_tone_colors(tone);
        let status_label = sftp_transfer_status_label(&transfer.status);
        let error_message = match &transfer.status {
            SftpTransferStatus::Failed(message) => Some(message.clone()),
            _ => None,
        };
        let is_active = matches!(
            transfer.status,
            SftpTransferStatus::Queued | SftpTransferStatus::Running | SftpTransferStatus::Paused
        );
        let has_children = !transfer.children.is_empty();
        let expanded = transfer.expanded && has_children;

        let expand_control = if has_children {
            let expand_controller = controller.clone();
            sftp_transfer_action_button(
                sftp_transfer_action_id(tab_id, transfer_id, "expand"),
                if expanded {
                    AppIcon::ChevronDown
                } else {
                    AppIcon::Next
                },
                i18n::string(if expanded {
                    "sftp.tooltips.collapse_transfer_children"
                } else {
                    "sftp.tooltips.expand_transfer_children"
                }),
                false,
                move |_window, cx| {
                    expand_controller.update(cx, |controller, cx| {
                        controller.toggle_transfer_expanded(tab_id, transfer_id, cx);
                    });
                },
            )
            .into_any_element()
        } else {
            div().size(px(SFTP_TRANSFER_ACTION_SIZE)).into_any_element()
        };

        let transfer_actions = match &transfer.status {
            SftpTransferStatus::Queued | SftpTransferStatus::Running => {
                let pause_controller = controller.clone();
                let cancel_controller = controller.clone();
                h_flex()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .child(sftp_transfer_action_button(
                        sftp_transfer_action_id(tab_id, transfer_id, "pause"),
                        AppIcon::Pause,
                        i18n::string("sftp.tooltips.pause_transfer"),
                        false,
                        move |_window, cx| {
                            pause_controller.update(cx, |controller, cx| {
                                controller.pause_transfer(tab_id, transfer_id, cx);
                            });
                        },
                    ))
                    .child(sftp_transfer_action_button(
                        sftp_transfer_action_id(tab_id, transfer_id, "cancel"),
                        AppIcon::Close,
                        i18n::string("sftp.tooltips.cancel_transfer"),
                        true,
                        move |_window, cx| {
                            cancel_controller.update(cx, |controller, cx| {
                                controller.cancel_transfer(tab_id, transfer_id, cx);
                            });
                        },
                    ))
                    .into_any_element()
            }
            SftpTransferStatus::Paused => {
                let resume_controller = controller.clone();
                let cancel_controller = controller.clone();
                h_flex()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .child(sftp_transfer_action_button(
                        sftp_transfer_action_id(tab_id, transfer_id, "resume"),
                        AppIcon::Play,
                        i18n::string("sftp.tooltips.resume_transfer"),
                        false,
                        move |_window, cx| {
                            resume_controller.update(cx, |controller, cx| {
                                controller.resume_transfer(tab_id, transfer_id, cx);
                            });
                        },
                    ))
                    .child(sftp_transfer_action_button(
                        sftp_transfer_action_id(tab_id, transfer_id, "cancel"),
                        AppIcon::Close,
                        i18n::string("sftp.tooltips.cancel_transfer"),
                        true,
                        move |_window, cx| {
                            cancel_controller.update(cx, |controller, cx| {
                                controller.cancel_transfer(tab_id, transfer_id, cx);
                            });
                        },
                    ))
                    .into_any_element()
            }
            SftpTransferStatus::Done
            | SftpTransferStatus::Cancelled
            | SftpTransferStatus::Failed(_) => {
                let delete_controller = controller;
                h_flex()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .child(sftp_transfer_action_button(
                        sftp_transfer_action_id(tab_id, transfer_id, "remove"),
                        AppIcon::Trash,
                        i18n::string("sftp.tooltips.remove_transfer"),
                        false,
                        move |_window, cx| {
                            delete_controller.update(cx, |controller, cx| {
                                controller.remove_transfer_record(tab_id, transfer_id, cx);
                            });
                        },
                    ))
                    .into_any_element()
            }
        };

        let child_rows = self.render_sftp_transfer_children(tab_id, transfer);

        card_surface(
            if is_active {
                roles.surface_container_high
            } else {
                roles.surface_container
            },
            14.0,
        )
        .id(SharedString::from(format!(
            "sftp-transfer-card-{tab_id}-{}",
            transfer_id.0
        )))
        .w_full()
        .flex_shrink_0()
        .p_3()
        .child(
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .min_w(px(0.0))
                        .items_center()
                        .gap_2()
                        .child(expand_control)
                        .child(
                            div()
                                .size(px(34.0))
                                .flex_shrink_0()
                                .rounded(px(10.0))
                                .bg(rgb(badge_background))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(rgb(accent))
                                .child(Icon::new(direction_icon).small()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(miaominal_settings::FontSize::Subheading.scaled())
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(roles.on_surface))
                                .child(display_name),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "sftp-transfer-profile-{tab_id}-{}",
                                    transfer_id.0
                                )))
                                .max_w(px(160.0))
                                .flex_shrink_0()
                                .overflow_hidden()
                                .tooltip(sftp_path_tooltip(profile_label))
                                .child(badge(
                                    profile_display,
                                    roles.surface_container_highest,
                                    roles.on_surface_variant,
                                )),
                        )
                        .child(badge(status_label, badge_background, badge_foreground))
                        .child(transfer_actions),
                )
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "sftp-transfer-route-{tab_id}-{}",
                            transfer_id.0
                        )))
                        .w_full()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(roles.on_surface_variant))
                        .tooltip(sftp_path_tooltip(route.clone()))
                        .child(route),
                )
                .when_some(error_message, |this, message| {
                    this.child(
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .rounded(px(8.0))
                            .bg(rgb(roles.error_container))
                            .px_2()
                            .py_1()
                            .text_size(miaominal_settings::FontSize::Body.scaled())
                            .text_color(rgb(roles.on_error_container))
                            .child(message),
                    )
                })
                .child(
                    Progress::new(format!("sftp-transfer-progress-{tab_id}-{}", transfer_id.0))
                        .with_size(gpui_component::Size::Small)
                        .value(progress_metrics.value)
                        .loading(progress_metrics.loading)
                        .color(rgb(accent)),
                )
                .child(
                    div()
                        .w_full()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .text_color(rgb(roles.on_surface_variant))
                        .child(progress_details),
                )
                .when(expanded, |this| this.child(child_rows)),
        )
        .into_any_element()
    }

    fn render_sftp_transfer_children(
        &self,
        tab_id: TabId,
        transfer: &SftpTransferRow,
    ) -> gpui::AnyElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let transfer_id = transfer.transfer_id;
        let mut rows = v_flex().w_full().gap_1().pl(px(38.0));
        let omitted_child_count = transfer.omitted_child_count();

        if omitted_child_count > 0 {
            let shown = transfer.children.len().to_string();
            let total = transfer.child_count.to_string();
            rows = rows.child(
                div()
                    .w_full()
                    .py_1()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(roles.on_surface_variant))
                    .child(i18n::string_args(
                        "sftp.ui.transfer_children_truncated",
                        &[("shown", &shown), ("total", &total)],
                    )),
            );
        }

        for child in &transfer.children {
            let done = matches!(child.status, SftpTransferChildStatus::Done);
            let loading = matches!(child.status, SftpTransferChildStatus::Running);
            let metrics =
                sftp_progress_metrics(child.bytes_complete, child.bytes_total, done, loading);
            let progress_label = child.bytes_total.map_or_else(
                || format_byte_size(Some(child.bytes_complete)).to_string(),
                |total| {
                    format!(
                        "{} / {}",
                        format_byte_size(Some(child.bytes_complete)),
                        format_byte_size(Some(total))
                    )
                },
            );
            let progress_label = metrics
                .percent
                .map(|percent| format!("{progress_label} · {percent}%"))
                .unwrap_or(progress_label);
            let tone = sftp_transfer_child_status_tone(&child.status);
            let (accent, badge_background, badge_foreground) = sftp_transfer_tone_colors(tone);
            let status_label = sftp_transfer_child_status_label(&child.status);
            let error_message = match &child.status {
                SftpTransferChildStatus::Failed(message) => Some(message.clone()),
                _ => None,
            };
            let child_path = child.relative_path.clone();

            rows = rows.child(
                card_surface(roles.surface_container_low, 10.0)
                    .id(SharedString::from(format!(
                        "sftp-transfer-child-{tab_id}-{}-{}",
                        transfer_id.0, child.child_id.0
                    )))
                    .w_full()
                    .p_2()
                    .child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .size(px(24.0))
                                            .flex_shrink_0()
                                            .rounded(px(8.0))
                                            .bg(rgb(badge_background))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_color(rgb(accent))
                                            .child(Icon::new(AppIcon::File).small()),
                                    )
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "sftp-transfer-child-path-{tab_id}-{}-{}",
                                                transfer_id.0, child.child_id.0
                                            )))
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .tooltip(sftp_path_tooltip(child_path.clone()))
                                            .text_size(miaominal_settings::FontSize::Body.scaled())
                                            .text_color(rgb(roles.on_surface))
                                            .child(child_path),
                                    )
                                    .child(badge(status_label, badge_background, badge_foreground)),
                            )
                            .when_some(error_message, |this, message| {
                                this.child(
                                    div()
                                        .w_full()
                                        .text_size(miaominal_settings::FontSize::Body.scaled())
                                        .text_color(rgb(roles.error))
                                        .child(message),
                                )
                            })
                            .child(
                                Progress::new(format!(
                                    "sftp-transfer-child-progress-{tab_id}-{}-{}",
                                    transfer_id.0, child.child_id.0
                                ))
                                .with_size(gpui_component::Size::Small)
                                .value(metrics.value)
                                .loading(metrics.loading)
                                .color(rgb(accent)),
                            )
                            .child(
                                div()
                                    .text_size(miaominal_settings::FontSize::Body.scaled())
                                    .text_color(rgb(roles.on_surface_variant))
                                    .child(progress_label),
                            ),
                    ),
            );
        }

        rows.into_any_element()
    }

    fn cache_sftp_browser_container_width(
        &mut self,
        tab_id: TabId,
        width: Pixels,
        cx: &mut Context<Self>,
    ) {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };

        if tab.layout.browser_container_width != width {
            tab.layout.browser_container_width = width;
            drop(tab);
            cx.notify();
        }
    }

    fn cache_sftp_page_container_height(
        &mut self,
        tab_id: TabId,
        height: Pixels,
        cx: &mut Context<Self>,
    ) {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };

        if tab.layout.page_container_height != height {
            tab.layout.page_container_height = height;
            drop(tab);
            cx.notify();
        }
    }

    fn start_sftp_split_drag(
        &mut self,
        tab_id: TabId,
        divider: SftpSplitDivider,
        initial_pointer: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };

        let (initial_flex_a, container_size) = match divider {
            SftpSplitDivider::BrowserPanels => {
                let flex_a = sftp_local_panel_flex(&tab);
                (
                    flex_a,
                    sftp_usable_container_size(tab.layout.browser_container_width),
                )
            }
            SftpSplitDivider::ProgressCenter => {
                let flex_a = sftp_browser_area_flex(&tab);
                (
                    flex_a,
                    sftp_usable_container_size(tab.layout.page_container_height),
                )
            }
        };

        tab.layout.drag = Some(SftpSplitDragState {
            divider,
            initial_pointer,
            initial_flex_a,
            container_size,
        });
        drop(tab);
        cx.notify();
    }

    fn update_sftp_split_drag(&mut self, tab_id: TabId, pointer: f32, cx: &mut Context<Self>) {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };
        let Some(drag) = tab.layout.drag.clone() else {
            return;
        };

        let delta_flex = if drag.container_size > 0.0 {
            (pointer - drag.initial_pointer) / drag.container_size
        } else {
            0.0
        };

        match drag.divider {
            SftpSplitDivider::BrowserPanels => {
                let next_flex = clamp_sftp_local_panel_flex(
                    tab.layout.browser_container_width,
                    drag.initial_flex_a + delta_flex,
                );
                if tab.layout.local_panel_flex != Some(next_flex) {
                    tab.layout.local_panel_flex = Some(next_flex);
                    drop(tab);
                    cx.notify();
                }
            }
            SftpSplitDivider::ProgressCenter => {
                let next_flex = clamp_sftp_browser_area_flex(
                    tab.layout.page_container_height,
                    drag.initial_flex_a + delta_flex,
                );
                if tab.layout.browser_area_flex != Some(next_flex) {
                    tab.layout.browser_area_flex = Some(next_flex);
                    drop(tab);
                    cx.notify();
                }
            }
        }
    }

    fn finish_sftp_split_drag(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };

        if tab.layout.drag.take().is_some() {
            drop(tab);
            cx.notify();
        }
    }

    fn finish_sftp_page_pointer_drag(
        &mut self,
        tab_id: TabId,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let is_split_dragging = self
            .tab(tab_id)
            .is_some_and(|tab| tab.layout.drag.is_some());

        if is_split_dragging {
            self.finish_sftp_split_drag(tab_id, cx);
            return true;
        }

        self.finish_active_drag_selection(tab_id, position, cx)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the tab renderer needs explicit entity, tab, focus, layout, window, and GPUI context inputs"
    )]
    pub(in crate::ui::shell) fn render_sftp_page_for_tab(
        &mut self,
        entity: Entity<Self>,
        tab_id: TabId,
        ordered_tab_ids: &[TabId],
        preferred_tab_id: Option<TabId>,
        fallback_section: SidebarSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.render_sftp_page_content(
            entity,
            tab_id,
            ordered_tab_ids,
            preferred_tab_id,
            fallback_section,
            window,
            cx,
        )
    }

    pub(in crate::ui::shell) fn render_sftp_remote_browser_panel(
        &self,
        controller: Entity<Self>,
        tab_id: TabId,
        sftp_tab: &SftpTabState,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let extended = material.extended;

        let remote_path_bar = if self.remote_path_editing() {
            let up_controller = controller.clone();
            sftp_path_bar(
                sftp_path_input_shell(&self.remote_path_input()),
                false,
                move |_window, cx| {
                    up_controller.update(cx, |controller, cx| {
                        controller.navigate_remote_up(tab_id, cx);
                    });
                },
                |_window, _cx| {},
            )
            .into_any_element()
        } else {
            let breadcrumb_controller = controller.clone();
            let up_controller = controller.clone();
            let edit_controller = controller.clone();
            sftp_path_bar(
                sftp_path_breadcrumb_shell(
                    SharedString::from(format!("session-remote-sftp-path-{tab_id}")),
                    build_remote_sftp_breadcrumb(
                        &sftp_tab.remote_path,
                        breadcrumb_controller,
                        tab_id,
                    ),
                    sftp_tab.remote_path.clone(),
                ),
                true,
                move |_window, cx| {
                    up_controller.update(cx, |controller, cx| {
                        controller.navigate_remote_up(tab_id, cx);
                    });
                },
                move |_window, cx| {
                    edit_controller.update(cx, |controller, cx| {
                        controller.set_path_editing(SftpBrowserSide::Remote, true, cx);
                    });
                },
            )
            .into_any_element()
        };

        let remote_toolbar = h_flex()
            .items_center()
            .gap(px(SFTP_ACTION_BUTTON_GAP))
            .child(sftp_toolbar_button(
                AppIcon::Rotate,
                i18n::string("sftp.tooltips.refresh_remote"),
                {
                    let controller = controller.clone();
                    move |_window, cx| {
                        let path = controller
                            .read(cx)
                            .tab(tab_id)
                            .map(|tab| tab.remote_path.clone())
                            .unwrap_or_else(|| ".".into());
                        controller.update(cx, |controller, cx| {
                            controller.request_remote_directory(tab_id, path, cx);
                        });
                    }
                },
            ))
            .child(sftp_toolbar_button(
                AppIcon::Plus,
                i18n::string("sftp.tooltips.create_directory"),
                {
                    let controller = controller.clone();
                    move |window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.begin_create_directory(tab_id, window, cx);
                        });
                    }
                },
            ))
            .child(sftp_toolbar_button(
                AppIcon::Download,
                i18n::string("sftp.tooltips.download_selected"),
                {
                    let controller = controller.clone();
                    move |window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.queue_download_selected(tab_id, true, window, cx);
                        });
                    }
                },
            ))
            .into_any_element();

        let table_row_height = gpui_component::Size::Small.table_row_height();
        let remote_table_bounds = Rc::new(RefCell::new(None));
        let remote_sftp_table = self.remote_table();
        let remote_table_for_menu = self.remote_table();
        let remote_selected_count = sftp_tab.selected_remote_paths.len();

        let remote_list = div()
            .id(("sftp-remote-table-wrap", tab_id.raw()))
            .group("sftp-remote-drop")
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .on_prepaint({
                let remote_table_bounds = remote_table_bounds.clone();
                let table = self.remote_table();
                move |bounds, _, cx| {
                    *remote_table_bounds.borrow_mut() = Some(bounds);
                    table.update(cx, |table, cx| {
                        if table.delegate_mut().set_available_width(bounds.size.width) {
                            cx.notify();
                        }
                    });
                }
            })
            .on_mouse_down(MouseButton::Left, {
                let controller = controller.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event: &MouseDownEvent, _window, cx| {
                    if cx.has_active_drag() {
                        return;
                    }
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        controller.begin_drag_selection(
                            tab_id,
                            SftpBrowserSide::Remote,
                            event.position,
                            bounds,
                            table_row_height,
                            cx,
                        );
                    });
                }
            })
            .on_mouse_move({
                let controller = controller.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event: &MouseMoveEvent, _window, cx| {
                    if event.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    if cx.has_active_drag() {
                        return;
                    }
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        if controller.update_drag_selection(
                            tab_id,
                            SftpBrowserSide::Remote,
                            event.position,
                            bounds,
                            table_row_height,
                            cx,
                        ) {
                            cx.stop_propagation();
                        }
                    });
                }
            })
            .capture_any_mouse_up({
                let controller = controller.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event: &MouseUpEvent, _window, cx| {
                    if event.button != MouseButton::Left {
                        return;
                    }
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        if controller.finish_drag_selection(
                            tab_id,
                            SftpBrowserSide::Remote,
                            event.position,
                            bounds,
                            table_row_height,
                            cx,
                        ) {
                            cx.stop_propagation();
                        }
                    });
                }
            })
            .on_click({
                let controller = controller.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event, _window, cx| {
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        controller.handle_blank_click(
                            tab_id,
                            SftpBrowserSide::Remote,
                            event.position(),
                            bounds,
                            table_row_height,
                            cx,
                        );
                    });
                }
            })
            .child(
                DataTable::new(&self.remote_table())
                    .with_size(gpui_component::Size::Small)
                    .bordered(false)
                    .scrollbar_visible(true, true),
            )
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                if !event.modifiers.shift {
                    return;
                }
                let delta = event.delta.pixel_delta(window.line_height());
                if delta.y == px(0.) {
                    return;
                }
                remote_sftp_table.update(cx, |state, cx| {
                    let mut offset = state.horizontal_scroll_handle.offset();
                    offset.x += delta.y;
                    state.horizontal_scroll_handle.set_offset(offset);
                    cx.notify();
                });
                cx.stop_propagation();
            })
            .when_some(sftp_tab.remote_drag_selection, |this, drag| {
                let bounds = sftp_drag_selection_overlay_bounds(drag, table_row_height);
                this.child(
                    div()
                        .absolute()
                        .left(bounds.origin.x)
                        .top(bounds.origin.y)
                        .w(bounds.size.width)
                        .h(bounds.size.height)
                        .border_1()
                        .border_color(color_with_alpha(extended.info.color, 0x80))
                        .bg(color_with_alpha(extended.info.color, 0x24)),
                )
            })
            .on_drop::<ExternalPaths>({
                let controller = controller.clone();
                move |paths: &ExternalPaths, _window, cx| {
                    let local_paths: Vec<PathBuf> = paths.paths().to_vec();
                    controller.update(cx, |controller, cx| {
                        controller.queue_upload_paths(tab_id, local_paths, cx);
                    });
                }
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .invisible()
                    .group_drag_over::<ExternalPaths>("sftp-remote-drop", |style| style.visible())
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(color_with_alpha(roles.primary, 0x20))
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Subheading.scaled())
                            .text_color(rgb(roles.on_primary))
                            .font_weight(FontWeight::MEDIUM)
                            .child(i18n::string("sftp.ui.drop_to_upload")),
                    ),
            )
            .into_any_element();

        sftp_browser_section(SftpBrowserSection {
            section_id: SharedString::from(format!("remote-sftp-section-{tab_id}")).into(),
            title: i18n::string("sftp.ui.remote_section").into(),
            show_title: true,
            icon: AppIcon::FolderSymlink,
            item_count: sftp_tab.remote_entries.len(),
            selected_count: remote_selected_count,
            path_bar: remote_path_bar,
            toolbar: remote_toolbar,
            content: remote_list,
            menu_builder: {
                let controller = controller.clone();
                let remote_table_for_menu = remote_table_for_menu.clone();
                move |menu, _window: &mut Window, cx: &mut Context<PopupMenu>| {
                    let is_header = remote_table_for_menu.update(cx, |state, _| {
                        state.delegate_mut().take_col_header_right_clicked()
                    });
                    if is_header {
                        return menu;
                    }
                    build_remote_sftp_context_menu(menu, controller.clone(), tab_id, true, cx)
                }
            },
        })
        .into_any_element()
    }

    fn render_missing_sftp_page_content(section: SidebarSection) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );

        div()
            .size_full()
            .rounded(px(24.0))
            .bg(rgb(roles.surface_container_highest))
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        v_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(miaominal_settings::FontSize::DisplayLarge.scaled())
                                    .text_color(rgb(roles.on_surface))
                                    .child(section.title()),
                            )
                            .child(
                                div()
                                    .max_w(px(420.0))
                                    .text_size(miaominal_settings::FontSize::Input.scaled())
                                    .line_height(miaominal_settings::scaled_line_height(18.0))
                                    .text_color(rgb(text_muted))
                                    .child(section.subtitle()),
                            )
                            .child(badge(
                                crate::ui::i18n::string("navigation.placeholder_panel"),
                                roles.surface_container_high,
                                roles.on_surface_variant,
                            )),
                    ),
            )
            .into_any_element()
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the content renderer needs explicit entity, tab, focus, layout, window, and GPUI context inputs"
    )]
    pub(in crate::ui::shell) fn render_sftp_page_content(
        &mut self,
        entity: Entity<Self>,
        tab_id: TabId,
        ordered_tab_ids: &[TabId],
        preferred_tab_id: Option<TabId>,
        fallback_section: SidebarSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let extended = material.extended;
        let progress_center_visibility = self.tab_progress_render_visibility(tab_id, window);
        let Some(sftp_tab) = self.tab(tab_id).map(|tab| tab.clone()) else {
            return Self::render_missing_sftp_page_content(fallback_section);
        };

        let local_panel_flex = sftp_local_panel_flex(&sftp_tab);
        let browser_area_flex = sftp_browser_area_flex(&sftp_tab);
        let progress_center_visibility = progress_center_visibility.unwrap_or(0.0);
        let progress_center_visible = progress_center_visibility > 0.0;
        let progress_center_footer_flex = (1.0 - browser_area_flex) * progress_center_visibility;
        let browser_panel_flex = 1.0 - progress_center_footer_flex;
        let local_path_bar = if self.local_path_editing() {
            let up_controller = entity.clone();
            sftp_path_bar(
                sftp_path_input_shell(&self.local_path_input()),
                false,
                move |_window, cx| {
                    up_controller.update(cx, |controller, cx| {
                        controller.navigate_local_up(tab_id, cx);
                    });
                },
                |_window, _cx| {},
            )
            .into_any_element()
        } else {
            let breadcrumb_controller = entity.clone();
            let up_controller = entity.clone();
            let edit_controller = entity.clone();
            sftp_path_bar(
                sftp_path_breadcrumb_shell(
                    SharedString::from(format!("local-sftp-path-{tab_id}")),
                    build_local_sftp_breadcrumb(
                        &sftp_tab.local_path,
                        breadcrumb_controller,
                        tab_id,
                    ),
                    SftpController::display_local_path(&sftp_tab.local_path),
                ),
                true,
                move |_window, cx| {
                    up_controller.update(cx, |controller, cx| {
                        controller.navigate_local_up(tab_id, cx);
                    });
                },
                move |_window, cx| {
                    edit_controller.update(cx, |controller, cx| {
                        controller.set_path_editing(SftpBrowserSide::Local, true, cx);
                    });
                },
            )
            .into_any_element()
        };
        let remote_path_bar = if self.remote_path_editing() {
            let up_controller = entity.clone();
            sftp_path_bar(
                sftp_path_input_shell(&self.remote_path_input()),
                false,
                move |_window, cx| {
                    up_controller.update(cx, |controller, cx| {
                        controller.navigate_remote_up(tab_id, cx);
                    });
                },
                |_window, _cx| {},
            )
            .into_any_element()
        } else {
            let breadcrumb_controller = entity.clone();
            let up_controller = entity.clone();
            let edit_controller = entity.clone();
            sftp_path_bar(
                sftp_path_breadcrumb_shell(
                    SharedString::from(format!("remote-sftp-path-{tab_id}")),
                    build_remote_sftp_breadcrumb(
                        &sftp_tab.remote_path,
                        breadcrumb_controller,
                        tab_id,
                    ),
                    sftp_tab.remote_path.clone(),
                ),
                true,
                move |_window, cx| {
                    up_controller.update(cx, |controller, cx| {
                        controller.navigate_remote_up(tab_id, cx);
                    });
                },
                move |_window, cx| {
                    edit_controller.update(cx, |controller, cx| {
                        controller.set_path_editing(SftpBrowserSide::Remote, true, cx);
                    });
                },
            )
            .into_any_element()
        };

        let table_row_height = gpui_component::Size::Small.table_row_height();
        let local_table_bounds = Rc::new(RefCell::new(None));
        let remote_table_bounds = Rc::new(RefCell::new(None));
        let local_sftp_table = self.local_table();
        let remote_sftp_table = self.remote_table();
        let local_table_for_menu = self.local_table();
        let remote_table_for_menu = self.remote_table();
        let local_selected_count = sftp_tab.selected_local_paths.len();
        let remote_selected_count = sftp_tab.selected_remote_paths.len();

        let local_toolbar = h_flex()
            .items_center()
            .gap(px(SFTP_ACTION_BUTTON_GAP))
            .child(sftp_toolbar_button(
                AppIcon::Rotate,
                i18n::string("sftp.tooltips.refresh_local"),
                {
                    let controller = entity.clone();
                    move |_window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.refresh_local_directory(tab_id, cx);
                        });
                    }
                },
            ))
            .child(sftp_toolbar_button(
                AppIcon::Upload,
                i18n::string("sftp.tooltips.upload_selected"),
                {
                    let controller = entity.clone();
                    move |_window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.queue_upload_selected(tab_id, cx);
                        });
                    }
                },
            ))
            .into_any_element();

        let remote_toolbar = h_flex()
            .items_center()
            .gap(px(SFTP_ACTION_BUTTON_GAP))
            .child(sftp_toolbar_button(
                AppIcon::Rotate,
                i18n::string("sftp.tooltips.refresh_remote"),
                {
                    let controller = entity.clone();
                    move |_window, cx| {
                        let path = controller
                            .read(cx)
                            .tab(tab_id)
                            .map(|tab| tab.remote_path.clone())
                            .unwrap_or_else(|| ".".into());
                        controller.update(cx, |controller, cx| {
                            controller.request_remote_directory(tab_id, path, cx);
                        });
                    }
                },
            ))
            .child(sftp_toolbar_button(
                AppIcon::Plus,
                i18n::string("sftp.tooltips.create_directory"),
                {
                    let controller = entity.clone();
                    move |window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.begin_create_directory(tab_id, window, cx);
                        });
                    }
                },
            ))
            .child(sftp_toolbar_button(
                AppIcon::Download,
                i18n::string("sftp.tooltips.download_selected"),
                {
                    let controller = entity.clone();
                    move |window, cx| {
                        controller.update(cx, |controller, cx| {
                            controller.queue_download_selected(tab_id, false, window, cx);
                        });
                    }
                },
            ))
            .into_any_element();

        let local_list = div()
            .id(("sftp-local-table-wrap", tab_id.raw()))
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .on_prepaint({
                let local_table_bounds = local_table_bounds.clone();
                let table = self.local_table();
                move |bounds, _, cx| {
                    *local_table_bounds.borrow_mut() = Some(bounds);
                    table.update(cx, |table, cx| {
                        if table.delegate_mut().set_available_width(bounds.size.width) {
                            cx.notify();
                        }
                    });
                }
            })
            .on_mouse_down(MouseButton::Left, {
                let controller = entity.clone();
                let local_table_bounds = local_table_bounds.clone();
                move |event: &MouseDownEvent, _window, cx| {
                    if cx.has_active_drag() {
                        return;
                    }
                    let Some(bounds) = *local_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        controller.begin_drag_selection(
                            tab_id,
                            SftpBrowserSide::Local,
                            event.position,
                            bounds,
                            table_row_height,
                            cx,
                        );
                    });
                }
            })
            .on_mouse_move({
                let controller = entity.clone();
                let local_table_bounds = local_table_bounds.clone();
                move |event: &MouseMoveEvent, _window, cx| {
                    if event.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    if cx.has_active_drag() {
                        return;
                    }
                    let Some(bounds) = *local_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        if controller.update_drag_selection(
                            tab_id,
                            SftpBrowserSide::Local,
                            event.position,
                            bounds,
                            table_row_height,
                            cx,
                        ) {
                            cx.stop_propagation();
                        }
                    });
                }
            })
            .capture_any_mouse_up({
                let controller = entity.clone();
                let local_table_bounds = local_table_bounds.clone();
                move |event: &MouseUpEvent, _window, cx| {
                    if event.button != MouseButton::Left {
                        return;
                    }
                    let Some(bounds) = *local_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        if controller.finish_drag_selection(
                            tab_id,
                            SftpBrowserSide::Local,
                            event.position,
                            bounds,
                            table_row_height,
                            cx,
                        ) {
                            cx.stop_propagation();
                        }
                    });
                }
            })
            .on_click({
                let controller = entity.clone();
                let local_table_bounds = local_table_bounds.clone();
                move |event, _window, cx| {
                    let Some(bounds) = *local_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        controller.handle_blank_click(
                            tab_id,
                            SftpBrowserSide::Local,
                            event.position(),
                            bounds,
                            table_row_height,
                            cx,
                        );
                    });
                }
            })
            .child(
                DataTable::new(&self.local_table())
                    .with_size(gpui_component::Size::Small)
                    .bordered(false)
                    .scrollbar_visible(true, true),
            )
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                if !event.modifiers.shift {
                    return;
                }
                let delta = event.delta.pixel_delta(window.line_height());
                if delta.y == px(0.) {
                    return;
                }
                local_sftp_table.update(cx, |state, cx| {
                    let mut offset = state.horizontal_scroll_handle.offset();
                    offset.x += delta.y;
                    state.horizontal_scroll_handle.set_offset(offset);
                    cx.notify();
                });
                cx.stop_propagation();
            })
            .when_some(sftp_tab.local_drag_selection, |this, drag| {
                let bounds = sftp_drag_selection_overlay_bounds(drag, table_row_height);
                this.child(
                    div()
                        .absolute()
                        .left(bounds.origin.x)
                        .top(bounds.origin.y)
                        .w(bounds.size.width)
                        .h(bounds.size.height)
                        .border_1()
                        .border_color(color_with_alpha(extended.info.color, 0x80))
                        .bg(color_with_alpha(extended.info.color, 0x24)),
                )
            })
            .into_any_element();

        let remote_list = div()
            .id(("sftp-remote-table-wrap", tab_id.raw()))
            .group("sftp-remote-drop")
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .on_prepaint({
                let remote_table_bounds = remote_table_bounds.clone();
                let table = self.remote_table();
                move |bounds, _, cx| {
                    *remote_table_bounds.borrow_mut() = Some(bounds);
                    table.update(cx, |table, cx| {
                        if table.delegate_mut().set_available_width(bounds.size.width) {
                            cx.notify();
                        }
                    });
                }
            })
            .on_mouse_down(MouseButton::Left, {
                let controller = entity.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event: &MouseDownEvent, _window, cx| {
                    if cx.has_active_drag() {
                        return;
                    }
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        controller.begin_drag_selection(
                            tab_id,
                            SftpBrowserSide::Remote,
                            event.position,
                            bounds,
                            table_row_height,
                            cx,
                        );
                    });
                }
            })
            .on_mouse_move({
                let controller = entity.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event: &MouseMoveEvent, _window, cx| {
                    if event.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    if cx.has_active_drag() {
                        return;
                    }
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        if controller.update_drag_selection(
                            tab_id,
                            SftpBrowserSide::Remote,
                            event.position,
                            bounds,
                            table_row_height,
                            cx,
                        ) {
                            cx.stop_propagation();
                        }
                    });
                }
            })
            .capture_any_mouse_up({
                let controller = entity.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event: &MouseUpEvent, _window, cx| {
                    if event.button != MouseButton::Left {
                        return;
                    }
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        if controller.finish_drag_selection(
                            tab_id,
                            SftpBrowserSide::Remote,
                            event.position,
                            bounds,
                            table_row_height,
                            cx,
                        ) {
                            cx.stop_propagation();
                        }
                    });
                }
            })
            .on_click({
                let controller = entity.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event, _window, cx| {
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    controller.update(cx, |controller, cx| {
                        controller.handle_blank_click(
                            tab_id,
                            SftpBrowserSide::Remote,
                            event.position(),
                            bounds,
                            table_row_height,
                            cx,
                        );
                    });
                }
            })
            .child(
                DataTable::new(&self.remote_table())
                    .with_size(gpui_component::Size::Small)
                    .bordered(false)
                    .scrollbar_visible(true, true),
            )
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                if !event.modifiers.shift {
                    return;
                }
                let delta = event.delta.pixel_delta(window.line_height());
                if delta.y == px(0.) {
                    return;
                }
                remote_sftp_table.update(cx, |state, cx| {
                    let mut offset = state.horizontal_scroll_handle.offset();
                    offset.x += delta.y;
                    state.horizontal_scroll_handle.set_offset(offset);
                    cx.notify();
                });
                cx.stop_propagation();
            })
            .when_some(sftp_tab.remote_drag_selection, |this, drag| {
                let bounds = sftp_drag_selection_overlay_bounds(drag, table_row_height);
                this.child(
                    div()
                        .absolute()
                        .left(bounds.origin.x)
                        .top(bounds.origin.y)
                        .w(bounds.size.width)
                        .h(bounds.size.height)
                        .border_1()
                        .border_color(color_with_alpha(extended.info.color, 0x80))
                        .bg(color_with_alpha(extended.info.color, 0x24)),
                )
            })
            .on_drop::<ExternalPaths>({
                let controller = entity.clone();
                move |paths: &ExternalPaths, _window, cx| {
                    let local_paths: Vec<PathBuf> = paths.paths().to_vec();
                    controller.update(cx, |controller, cx| {
                        controller.queue_upload_paths(tab_id, local_paths, cx);
                    });
                }
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .invisible()
                    .group_drag_over::<ExternalPaths>("sftp-remote-drop", |style| style.visible())
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(color_with_alpha(roles.primary, 0x20))
                    .child(
                        div()
                            .text_size(miaominal_settings::FontSize::Subheading.scaled())
                            .text_color(rgb(roles.on_primary))
                            .font_weight(FontWeight::MEDIUM)
                            .child(i18n::string("sftp.ui.drop_to_upload")),
                    ),
            )
            .into_any_element();

        let footer = self.render_sftp_progress_center(
            entity.clone(),
            format!("sftp-progress-center-{tab_id}"),
            ordered_tab_ids,
            preferred_tab_id,
        );

        let browser_panels = div()
            .flex()
            .flex_row()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .on_prepaint({
                let entity = entity.clone();
                move |bounds, _window, cx| {
                    let entity = entity.clone();
                    entity.update(cx, |this, cx| {
                        this.cache_sftp_browser_container_width(tab_id, bounds.size.width, cx);
                    });
                }
            })
            .child(
                div()
                    .flex_grow(1.0)
                    .flex_shrink(1.0)
                    .flex_basis(gpui::relative(local_panel_flex))
                    .h_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .child(sftp_browser_section(SftpBrowserSection {
                        section_id: SharedString::from(format!("local-sftp-section-{tab_id}"))
                            .into(),
                        title: i18n::string("sftp.ui.local_section").into(),
                        show_title: true,
                        icon: AppIcon::Computer,
                        item_count: sftp_tab.local_entries.len(),
                        selected_count: local_selected_count,
                        path_bar: local_path_bar,
                        toolbar: local_toolbar,
                        content: local_list,
                        menu_builder: {
                            let controller = entity.clone();
                            let local_table_for_menu = local_table_for_menu.clone();
                            move |menu, _window: &mut Window, cx: &mut Context<PopupMenu>| {
                                let is_header = local_table_for_menu.update(cx, |state, _| {
                                    state.delegate_mut().take_col_header_right_clicked()
                                });
                                if is_header {
                                    return menu;
                                }
                                build_local_sftp_context_menu(menu, controller.clone(), tab_id, cx)
                            }
                        },
                    })),
            )
            .child(sftp_split_bar(
                entity.clone(),
                tab_id,
                SftpSplitDivider::BrowserPanels,
                matches!(
                    sftp_tab.layout.drag.as_ref(),
                    Some(drag) if drag.divider == SftpSplitDivider::BrowserPanels
                ),
            ))
            .child(
                div()
                    .flex_grow(1.0)
                    .flex_shrink(1.0)
                    .flex_basis(gpui::relative(1.0 - local_panel_flex))
                    .h_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .child(sftp_browser_section(SftpBrowserSection {
                        section_id: SharedString::from(format!("remote-sftp-section-{tab_id}"))
                            .into(),
                        title: i18n::string("sftp.ui.remote_section").into(),
                        show_title: true,
                        icon: AppIcon::FolderSymlink,
                        item_count: sftp_tab.remote_entries.len(),
                        selected_count: remote_selected_count,
                        path_bar: remote_path_bar,
                        toolbar: remote_toolbar,
                        content: remote_list,
                        menu_builder: {
                            let controller = entity.clone();
                            let remote_table_for_menu = remote_table_for_menu.clone();
                            move |menu, _window: &mut Window, cx: &mut Context<PopupMenu>| {
                                let is_header = remote_table_for_menu.update(cx, |state, _| {
                                    state.delegate_mut().take_col_header_right_clicked()
                                });
                                if is_header {
                                    return menu;
                                }
                                build_remote_sftp_context_menu(
                                    menu,
                                    controller.clone(),
                                    tab_id,
                                    false,
                                    cx,
                                )
                            }
                        },
                    })),
            );

        div()
            .size_full()
            .relative()
            .bg(rgb(roles.surface_container))
            .capture_any_mouse_down(cx.listener(
                move |this, event: &MouseDownEvent, _window, cx| {
                    if event.button == MouseButton::Left {
                        let _ = this.finish_active_drag_selection(tab_id, event.position, cx);
                    }
                },
            ))
            .on_mouse_move(
                cx.listener(move |this, event: &MouseMoveEvent, _window, cx| {
                    if event.pressed_button != Some(MouseButton::Left) {
                        if this.finish_active_drag_selection(tab_id, event.position, cx) {
                            cx.stop_propagation();
                        }
                        return;
                    }

                    if let Some(divider) = this
                        .tab(tab_id)
                        .and_then(|tab| tab.layout.drag.as_ref().map(|drag| drag.divider))
                    {
                        let pointer = match divider {
                            SftpSplitDivider::BrowserPanels => f32::from(event.position.x),
                            SftpSplitDivider::ProgressCenter => f32::from(event.position.y),
                        };

                        this.update_sftp_split_drag(tab_id, pointer, cx);
                        cx.stop_propagation();
                        return;
                    }

                    if this.update_active_drag_selection(tab_id, event.position, cx) {
                        cx.stop_propagation();
                    }
                }),
            )
            .capture_any_mouse_up(cx.listener(move |this, event: &MouseUpEvent, _window, cx| {
                if event.button != MouseButton::Left {
                    return;
                }

                if this.finish_sftp_page_pointer_drag(tab_id, event.position, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseUpEvent, _window, cx| {
                    if this.finish_sftp_page_pointer_drag(tab_id, event.position, cx) {
                        cx.stop_propagation();
                    }
                }),
            )
            .child(
                div()
                    .size_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .on_prepaint({
                        let entity = entity.clone();
                        move |bounds, _window, cx| {
                            let entity = entity.clone();
                            entity.update(cx, |this, cx| {
                                this.cache_sftp_page_container_height(
                                    tab_id,
                                    bounds.size.height,
                                    cx,
                                );
                            });
                        }
                    })
                    .child(
                        v_flex()
                            .size_full()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .child(
                                div()
                                    .flex_grow(1.0)
                                    .flex_shrink(1.0)
                                    .flex_basis(gpui::relative(browser_panel_flex))
                                    .min_w(px(0.0))
                                    .min_h(px(0.0))
                                    .child(browser_panels),
                            )
                            .when(progress_center_visible, |this| {
                                this.child(
                                    div()
                                        .w_full()
                                        .h(px(SFTP_SPLIT_GAP * progress_center_visibility))
                                        .overflow_hidden()
                                        .opacity(progress_center_visibility)
                                        .child(sftp_split_bar(
                                            entity.clone(),
                                            tab_id,
                                            SftpSplitDivider::ProgressCenter,
                                            matches!(
                                                sftp_tab.layout.drag.as_ref(),
                                                Some(drag) if drag.divider == SftpSplitDivider::ProgressCenter
                                            ),
                                        )),
                                )
                                .child(
                                    div()
                                        .flex_grow(progress_center_visibility)
                                        .flex_shrink(1.0)
                                        .flex_basis(gpui::relative(progress_center_footer_flex))
                                        .min_w(px(0.0))
                                        .min_h(px(0.0))
                                        .overflow_hidden()
                                        .child(
                                            div()
                                                .relative()
                                                .size_full()
                                                .opacity(progress_center_visibility)
                                                .top(px(
                                                    (1.0 - progress_center_visibility)
                                                        * SFTP_PROGRESS_CENTER_SLIDE_OFFSET,
                                                ))
                                                .child(footer),
                                        ),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    pub(in crate::ui::shell) fn render_sftp_prompt_overlay(
        &self,
        entity: Entity<Self>,
        tab_id: TabId,
        prompt: &SftpPromptState,
        exit_progress: Option<f32>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let extended = material.extended;
        let (
            icon,
            icon_tint,
            title,
            confirm_label,
            supporting_text,
            is_overwrite_prompt,
            is_destructive_prompt,
        ) = match &prompt.kind {
            SftpPromptKind::CreateRemoteDirectory { .. } => (
                AppIcon::Folder,
                roles.primary,
                i18n::string("sftp.prompts.create_remote_directory.title"),
                i18n::string("sftp.prompts.create_remote_directory.confirm"),
                None,
                false,
                false,
            ),
            SftpPromptKind::ConfirmOverwrite { conflict_count, .. } => {
                let count_text = conflict_count.to_string();
                let msg = if *conflict_count == 1 {
                    i18n::string("sftp.prompts.confirm_overwrite.single_message")
                } else {
                    i18n::string_args(
                        "sftp.prompts.confirm_overwrite.multi_message",
                        &[("count", &count_text)],
                    )
                };
                (
                    AppIcon::Upload,
                    extended.warning.color,
                    i18n::string("sftp.prompts.confirm_overwrite.title"),
                    i18n::string("sftp.prompts.confirm_overwrite.confirm"),
                    Some(msg.to_string()),
                    true,
                    false,
                )
            }
            SftpPromptKind::ConfirmDelete { entries, .. } => {
                let msg = if entries.len() == 1 {
                    i18n::string_args(
                        "sftp.prompts.confirm_delete.single_message",
                        &[(
                            "name",
                            entries[0].0.rsplit('/').next().unwrap_or(&entries[0].0),
                        )],
                    )
                } else {
                    let count = entries.len().to_string();
                    i18n::string_args(
                        "sftp.prompts.confirm_delete.multi_message",
                        &[("count", &count)],
                    )
                };
                (
                    AppIcon::Trash,
                    roles.error,
                    i18n::string("sftp.prompts.confirm_delete.title"),
                    i18n::string("sftp.prompts.confirm_delete.confirm"),
                    Some(msg.to_string()),
                    false,
                    true,
                )
            }
        };

        let body = match &prompt.kind {
            SftpPromptKind::CreateRemoteDirectory { .. } => Some(
                HintedInput::new(&self.prompt_input())
                    .large()
                    .w_full()
                    .rounded(px(12.0))
                    .into_any_element(),
            ),
            SftpPromptKind::ConfirmOverwrite { .. } | SftpPromptKind::ConfirmDelete { .. } => None,
        };

        let cancel_button = basic_dialog_action_button(
            format!("sftp-prompt-cancel-{tab_id}"),
            i18n::string("sftp.prompts.cancel"),
            BasicDialogActionTone::Default,
        )
        .on_click({
            let controller = entity.clone();
            move |_, _, cx| {
                controller.update(cx, |controller, cx| {
                    controller.cancel_prompt(tab_id, cx);
                });
            }
        });

        let confirm_button = if is_destructive_prompt {
            basic_dialog_action_button(
                format!("sftp-prompt-confirm-{tab_id}"),
                confirm_label.clone(),
                BasicDialogActionTone::Destructive,
            )
        } else {
            basic_dialog_action_button(
                format!("sftp-prompt-confirm-{tab_id}"),
                confirm_label.clone(),
                BasicDialogActionTone::Default,
            )
        }
        .on_click({
            let controller = entity.clone();
            move |_, _, cx| {
                controller.update(cx, |controller, cx| {
                    controller.commit_prompt(tab_id, cx);
                });
            }
        });

        render_basic_dialog_with_config(
            format!("sftp-prompt-{tab_id}"),
            crate::ui::shell::support::BasicDialogConfig {
                title: title.to_string(),
                supporting_text,
                body,
                actions: h_flex()
                    .justify_end()
                    .gap_2()
                    .child(cancel_button)
                    .when(is_overwrite_prompt, |this| {
                        this.child(
                            basic_dialog_action_button(
                                format!("sftp-prompt-skip-{tab_id}"),
                                i18n::string("sftp.prompts.skip_existing"),
                                BasicDialogActionTone::Default,
                            )
                            .on_click({
                                let controller = entity.clone();
                                move |_, _, cx| {
                                    controller.update(cx, |controller, cx| {
                                        controller.skip_overwrite_prompt(tab_id, cx);
                                    });
                                }
                            }),
                        )
                    })
                    .child(confirm_button)
                    .into_any_element(),
                icon: Some(BasicDialogIcon {
                    icon,
                    tint: icon_tint,
                }),
                header_alignment: BasicDialogHeaderAlignment::Center,
                exit_progress,
            },
        )
    }
}
