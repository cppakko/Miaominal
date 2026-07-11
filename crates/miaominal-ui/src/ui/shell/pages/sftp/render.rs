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
        .unwrap_or_else(|| AppView::display_sftp_local_path(path))
}

fn build_local_sftp_breadcrumb(path: &Path, entity: Entity<AppView>, tab_id: usize) -> Breadcrumb {
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
            let click_entity = entity.clone();
            let target = ancestor.clone();
            item.on_click(move |_, _, cx| {
                let target = target.clone();
                click_entity.update(cx, |this, cx| {
                    this.navigate_sftp_local_to_path(tab_id, target.clone(), cx);
                });
            })
        };
        breadcrumb = breadcrumb.child(item);
    }

    breadcrumb
}

fn build_remote_sftp_breadcrumb(path: &str, entity: Entity<AppView>, tab_id: usize) -> Breadcrumb {
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
            accumulated = AppView::join_remote_path(&accumulated, segment);
            segments.push((accumulated.clone(), segment.to_string().into()));
        }
    } else {
        let mut accumulated = ".".to_string();
        segments.push((accumulated.clone(), ".".into()));
        for segment in current_path
            .split('/')
            .filter(|segment| !segment.is_empty() && *segment != ".")
        {
            accumulated = AppView::join_remote_path(&accumulated, segment);
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
            let click_entity = entity.clone();
            let target = target_path.clone();
            item.on_click(move |_, _, cx| {
                let target = target.clone();
                click_entity.update(cx, |this, cx| {
                    this.request_sftp_remote_directory(tab_id, target.clone(), cx);
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
    tab_id: usize,
    divider: SftpSplitDivider,
    is_dragging: bool,
    cx: &mut Context<AppView>,
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
        cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
            let pointer = match divider {
                SftpSplitDivider::BrowserPanels => f32::from(event.position.x),
                SftpSplitDivider::ProgressCenter => f32::from(event.position.y),
            };
            this.start_sftp_split_drag(tab_id, divider, pointer, cx);
        }),
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

fn sftp_progress_center_card(
    section_id: impl Into<ElementId>,
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
        .child(content_shell)
}

fn sftp_empty_transfer_summary(error: Option<String>) -> impl IntoElement {
    if let Some(error) = error {
        let material = miaominal_settings::current_theme().material;
        let extended = material.extended;
        return h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .px_3()
            .pb_2()
            .text_size(miaominal_settings::FontSize::Body.scaled())
            .child(
                div()
                    .size(px(7.0))
                    .rounded(px(999.0))
                    .bg(rgb(extended.warning.color)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_color(rgb(extended.warning.color))
                    .child(error),
            )
            .into_any_element();
    }

    shell_compact_empty_state(
        AppIcon::Forward,
        i18n::string("sftp.ui.transfer_idle"),
        SFTP_PROGRESS_CENTER_MIN_HEIGHT,
    )
    .into_any_element()
}

fn context_menu_local_sftp_entry(
    entity: &Entity<AppView>,
    tab_id: usize,
    cx: &App,
) -> Option<LocalSftpEntry> {
    let shell = entity.read(cx);
    let row_ix = shell
        .workspace_forms
        .sftp_browser
        .local_table
        .read(cx)
        .right_clicked_row()?;
    let path = shell
        .workspace_forms
        .sftp_browser
        .local_table
        .read(cx)
        .delegate()
        .row(row_ix)
        .map(|row| PathBuf::from(row.path.as_str()))?;

    shell
        .workspace_state
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .and_then(TabState::as_sftp)
        .and_then(|sftp| {
            sftp.local_entries
                .iter()
                .find(|entry| entry.path == path)
                .cloned()
        })
}

fn context_menu_remote_sftp_entry(
    entity: &Entity<AppView>,
    tab_id: usize,
    cx: &App,
) -> Option<SftpEntry> {
    let shell = entity.read(cx);
    let table = shell.workspace_forms.sftp_browser.remote_table.read(cx);
    let row_ix = table.right_clicked_row()?;
    let path = table.delegate().row(row_ix).map(|row| row.path.clone())?;
    shell.resolve_remote_sftp_entry(tab_id, &path, cx)
}

fn build_local_sftp_context_menu(
    menu: PopupMenu,
    entity: Entity<AppView>,
    tab_id: usize,
    cx: &App,
) -> PopupMenu {
    let mut menu = menu;

    if let Some(entry) = context_menu_local_sftp_entry(&entity, tab_id, cx) {
        if entry.is_directory {
            let open_entity = entity.clone();
            let open_path = entry.path.clone();
            menu = menu.item(PopupMenuItem::new(i18n::string("sftp.menu.open")).on_click(
                move |_, _, cx| {
                    let entity = open_entity.clone();
                    let path = open_path.clone();
                    entity.update(cx, |this, cx| {
                        this.select_sftp_local_path(tab_id, path.clone(), cx);
                        this.navigate_sftp_local_into_selected(tab_id, cx);
                    });
                },
            ));
        }

        let upload_entity = entity.clone();
        let upload_path = entry.path.clone();
        menu = menu
            .item(
                PopupMenuItem::new(i18n::string("sftp.menu.upload")).on_click(move |_, _, cx| {
                    let entity = upload_entity.clone();
                    let path = upload_path.clone();
                    entity.update(cx, |this, cx| {
                        let already_selected = this
                            .workspace_state
                            .tabs
                            .iter()
                            .find(|tab| tab.id == tab_id)
                            .and_then(TabState::as_sftp)
                            .map(|sftp| sftp.selected_local_paths.iter().any(|p| p == &path))
                            .unwrap_or(false);
                        if !already_selected {
                            this.select_sftp_local_path(tab_id, path.clone(), cx);
                        }
                        this.queue_sftp_upload_selected(tab_id, cx);
                    });
                }),
            )
            .item(PopupMenuItem::separator());
    }

    let up_entity = entity.clone();
    let refresh_entity = entity;
    menu.item(
        PopupMenuItem::new(i18n::string("sftp.menu.go_up")).on_click(move |_, _, cx| {
            let entity = up_entity.clone();
            entity.update(cx, |this, cx| {
                this.navigate_sftp_local_up(tab_id, cx);
            });
        }),
    )
    .item(
        PopupMenuItem::new(i18n::string("sftp.menu.refresh")).on_click(move |_, _, cx| {
            let entity = refresh_entity.clone();
            entity.update(cx, |this, cx| {
                this.refresh_sftp_local_directory(tab_id, cx);
            });
        }),
    )
}

fn build_remote_sftp_context_menu(
    menu: PopupMenu,
    entity: Entity<AppView>,
    tab_id: usize,
    cx: &App,
) -> PopupMenu {
    let mut menu = menu;

    if let Some(entry) = context_menu_remote_sftp_entry(&entity, tab_id, cx) {
        let is_single_selection = entity
            .read(cx)
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .map(|sftp| sftp.selected_remote_paths.len() == 1)
            .unwrap_or(false);

        if entry.kind == miaominal_sftp::SftpEntryKind::Directory {
            let open_entity = entity.clone();
            let open_path = entry.path.clone();
            menu = menu.item(PopupMenuItem::new(i18n::string("sftp.menu.open")).on_click(
                move |_, _, cx| {
                    let entity = open_entity.clone();
                    let path = open_path.clone();
                    entity.update(cx, |this, cx| {
                        this.select_sftp_remote_path(tab_id, path.clone(), cx);
                        this.navigate_sftp_remote_into_selected(tab_id, cx);
                    });
                },
            ));
        }

        let download_entity = entity.clone();
        let download_path = entry.path.clone();
        let edit_entity = entity.clone();
        let edit_path = entry.path.clone();
        let is_file = entry.kind != miaominal_sftp::SftpEntryKind::Directory;
        let rename_entity = entity.clone();
        let delete_entity = entity.clone();
        menu = menu.item(
            PopupMenuItem::new(i18n::string("sftp.menu.download")).on_click(move |_, _, cx| {
                let entity = download_entity.clone();
                let path = download_path.clone();
                entity.update(cx, |this, cx| {
                    let already_selected = this
                        .workspace_state
                        .tabs
                        .iter()
                        .find(|tab| tab.id == tab_id)
                        .and_then(TabState::as_sftp)
                        .map(|sftp| sftp.selected_remote_paths.iter().any(|p| p == &path))
                        .unwrap_or(false);
                    if !already_selected {
                        this.select_sftp_remote_path(tab_id, path.clone(), cx);
                    }
                    this.queue_sftp_download_selected(tab_id, cx);
                });
            }),
        );
        if is_single_selection && is_file {
            menu = menu.item(PopupMenuItem::new(i18n::string("sftp.menu.edit")).on_click(
                move |_, _, cx| {
                    let entity = edit_entity.clone();
                    let path = edit_path.clone();
                    entity.update(cx, |this, cx| {
                        this.open_remote_file_for_editing(tab_id, path, cx);
                    });
                },
            ));
        }
        if is_single_selection {
            menu = menu.item(
                PopupMenuItem::new(i18n::string("sftp.menu.rename")).on_click(
                    move |_, window, cx| {
                        let entity = rename_entity.clone();
                        entity.update(cx, |this, cx| {
                            this.begin_sftp_rename_selected(tab_id, window, cx);
                        });
                    },
                ),
            );
        }
        menu = menu
            .item(
                PopupMenuItem::new(i18n::string("sftp.menu.delete")).on_click(move |_, _, cx| {
                    let entity = delete_entity.clone();
                    entity.update(cx, |this, cx| {
                        this.delete_sftp_remote_selected(tab_id, cx);
                    });
                }),
            )
            .item(PopupMenuItem::separator());
    }

    let up_entity = entity.clone();
    let refresh_entity = entity.clone();
    let create_entity = entity;
    menu.item(
        PopupMenuItem::new(i18n::string("sftp.menu.go_up")).on_click(move |_, _, cx| {
            let entity = up_entity.clone();
            entity.update(cx, |this, cx| {
                this.navigate_sftp_remote_up(tab_id, cx);
            });
        }),
    )
    .item(
        PopupMenuItem::new(i18n::string("sftp.menu.refresh")).on_click(move |_, _, cx| {
            let entity = refresh_entity.clone();
            entity.update(cx, |this, cx| {
                let path = this
                    .workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
                    .and_then(TabState::as_sftp)
                    .map(|sftp| sftp.remote_path.clone())
                    .unwrap_or_else(|| ".".into());
                this.request_sftp_remote_directory(tab_id, path, cx);
            });
        }),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(i18n::string("sftp.menu.create_directory")).on_click(
            move |_, window, cx| {
                let entity = create_entity.clone();
                entity.update(cx, |this, cx| {
                    this.begin_sftp_create_directory(tab_id, window, cx);
                });
            },
        ),
    )
}

impl AppView {
    fn sftp_tab_mut(&mut self, tab_id: usize) -> Option<&mut SftpTabState> {
        self.workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
    }

    pub(in crate::ui::shell) fn toggle_active_sftp_progress_center(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let active_sftp = self
            .workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(|tab| {
                tab.as_sftp()
                    .map(|sftp| sftp.layout.progress_center_visible)
            });
        let side_panel_sftp = (self.panels.session_side_panel_open
            && self.panels.session_side_panel_view == SessionSidePanelView::Sftp)
            .then(|| self.session_side_panel_sftp_tab_id())
            .flatten()
            .and_then(|tab_id| {
                self.workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
                    .and_then(|tab| {
                        tab.as_sftp()
                            .map(|sftp| sftp.layout.progress_center_visible)
                    })
            });
        let Some(visible) = active_sftp.or(side_panel_sftp) else {
            return;
        };

        self.set_sftp_progress_center_visible(!visible, cx);
    }

    pub(in crate::ui::shell) fn set_sftp_progress_center_visible(
        &mut self,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        if self.apply_sftp_progress_center_visibility(visible) {
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn apply_sftp_progress_center_visibility(
        &mut self,
        visible: bool,
    ) -> bool {
        let phase = if visible {
            SftpProgressCenterTransitionPhase::Entering
        } else {
            SftpProgressCenterTransitionPhase::Exiting
        };
        let started_at = Instant::now();
        let mut changed = false;

        for tab in &mut self.workspace_state.tabs {
            let Some(sftp) = tab.as_sftp_mut() else {
                continue;
            };

            if sftp.layout.progress_center_visible == visible
                && sftp
                    .layout
                    .progress_center_transition
                    .as_ref()
                    .is_none_or(|transition| transition.phase == phase)
            {
                continue;
            }

            sftp.layout.progress_center_visible = visible;
            sftp.layout.progress_center_transition = Some(SftpProgressCenterTransition {
                phase,
                started_at,
                duration: CONTAINER_TRANSITION_DURATION,
            });

            if !visible
                && matches!(
                    sftp.layout.drag.as_ref(),
                    Some(drag) if drag.divider == SftpSplitDivider::ProgressCenter
                )
            {
                sftp.layout.drag = None;
            }
            changed = true;
        }

        changed
    }

    pub(in crate::ui::shell) fn sftp_progress_center_render_visibility(
        &mut self,
        tab_id: usize,
        window: &mut Window,
    ) -> Option<f32> {
        let tab = self.sftp_tab_mut(tab_id)?;

        let Some(transition) = tab.layout.progress_center_transition else {
            return tab.layout.progress_center_visible.then_some(1.0);
        };

        let duration_seconds = transition.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            tab.layout.progress_center_transition = None;
            return tab.layout.progress_center_visible.then_some(1.0);
        }

        let elapsed = Instant::now().saturating_duration_since(transition.started_at);
        let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
        let eased = progress * progress * (3.0 - 2.0 * progress);

        if progress >= 1.0 {
            tab.layout.progress_center_transition = None;
            return tab.layout.progress_center_visible.then_some(1.0);
        }

        window.request_animation_frame();

        Some(match transition.phase {
            SftpProgressCenterTransitionPhase::Entering => eased,
            SftpProgressCenterTransitionPhase::Exiting => 1.0 - eased,
        })
    }

    pub(in crate::ui::shell) fn render_sftp_progress_center(
        &self,
        entity: Entity<Self>,
        section_id: impl Into<ElementId>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let extended = material.extended;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );

        let transfers = self
            .workspace_state
            .tabs
            .iter()
            .filter_map(|tab| tab.as_sftp().map(|sftp| (tab.id, sftp)))
            .flat_map(|(tab_id, sftp)| {
                sftp.transfers
                    .iter()
                    .map(move |transfer| (tab_id, sftp, transfer))
            });

        let first_error = self
            .workspace_state
            .tabs
            .iter()
            .filter_map(TabState::as_sftp)
            .find_map(|sftp| sftp.last_error.clone());

        let mut rows = v_flex().w_full().gap_1().flex_shrink_0();
        let mut has_transfers = false;
        for (tab_id, sftp_tab, transfer) in transfers {
            has_transfers = true;
            let transfer_id = transfer.transfer_id;
            let profile_label = self
                .data
                .sessions
                .iter()
                .find(|profile| profile.id == sftp_tab.profile_id)
                .map(|profile| profile.name.as_str())
                .unwrap_or(sftp_tab.profile_id.as_str());
            let direction_icon = match transfer.direction {
                TransferDirection::Upload => AppIcon::Upload,
                TransferDirection::Download => AppIcon::Download,
            };
            let progress = transfer.bytes_total.map_or_else(
                || format_byte_size(Some(transfer.bytes_complete)).to_string(),
                |total| {
                    format!(
                        "{} / {}",
                        format_byte_size(Some(transfer.bytes_complete)),
                        format_byte_size(Some(total))
                    )
                },
            );
            let progress_value = match transfer.bytes_total {
                Some(total) if total > 0 => {
                    ((transfer.bytes_complete as f32 / total as f32) * 100.0).clamp(0.0, 100.0)
                }
                Some(_) if matches!(&transfer.status, SftpTransferStatus::Done) => 100.0,
                Some(_) => 0.0,
                None if matches!(&transfer.status, SftpTransferStatus::Done) => 100.0,
                None => 0.0,
            };
            let progress_loading = transfer.bytes_total.is_none()
                && matches!(
                    &transfer.status,
                    SftpTransferStatus::Queued | SftpTransferStatus::Running
                );
            let status_label = match &transfer.status {
                SftpTransferStatus::Queued => i18n::string("sftp.transfer_status.queued"),
                SftpTransferStatus::Running => i18n::string("sftp.transfer_status.running"),
                SftpTransferStatus::Paused => i18n::string("sftp.transfer_status.paused"),
                SftpTransferStatus::Done => i18n::string("sftp.transfer_status.done"),
                SftpTransferStatus::Cancelled => i18n::string("sftp.transfer_status.cancelled"),
                SftpTransferStatus::Failed(message) => {
                    i18n::string_args("sftp.transfer_status.failed", &[("message", message)])
                }
            };
            let accent = match &transfer.status {
                SftpTransferStatus::Queued => extended.warning.color,
                SftpTransferStatus::Running => extended.info.color,
                SftpTransferStatus::Paused => extended.warning.color,
                SftpTransferStatus::Done => extended.success.color,
                SftpTransferStatus::Cancelled => roles.on_surface_variant,
                SftpTransferStatus::Failed(_) => extended.warning.color,
            };
            let speed_label = if matches!(&transfer.status, SftpTransferStatus::Running) {
                transfer
                    .bytes_per_second
                    .map(|bps| format!("{}/s", format_byte_size(Some(bps))))
            } else {
                None
            };
            let transfer_actions = match &transfer.status {
                SftpTransferStatus::Queued | SftpTransferStatus::Running => {
                    let pause_entity = entity.clone();
                    let cancel_entity = entity.clone();
                    Some(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .flex_shrink_0()
                            .child(icon_button(
                                AppIcon::Pause,
                                22.0,
                                6.0,
                                Some(roles.surface_container_low),
                                Some(roles.on_surface_variant),
                                None,
                                move |_window, cx| {
                                    pause_entity.update(cx, |this, cx| {
                                        this.pause_sftp_transfer(tab_id, transfer_id, cx);
                                    });
                                },
                            ))
                            .child(icon_button(
                                AppIcon::Close,
                                22.0,
                                6.0,
                                Some(roles.surface_container_low),
                                Some(roles.on_surface_variant),
                                None,
                                move |_window, cx| {
                                    cancel_entity.update(cx, |this, cx| {
                                        this.cancel_sftp_transfer(tab_id, transfer_id, cx);
                                    });
                                },
                            ))
                            .into_any_element(),
                    )
                }
                SftpTransferStatus::Paused => {
                    let resume_entity = entity.clone();
                    let cancel_entity = entity.clone();
                    Some(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .flex_shrink_0()
                            .child(icon_button(
                                AppIcon::Play,
                                22.0,
                                6.0,
                                Some(roles.surface_container_low),
                                Some(roles.on_surface_variant),
                                None,
                                move |_window, cx| {
                                    resume_entity.update(cx, |this, cx| {
                                        this.resume_sftp_transfer(tab_id, transfer_id, cx);
                                    });
                                },
                            ))
                            .child(icon_button(
                                AppIcon::Close,
                                22.0,
                                6.0,
                                Some(roles.surface_container_low),
                                Some(roles.on_surface_variant),
                                None,
                                move |_window, cx| {
                                    cancel_entity.update(cx, |this, cx| {
                                        this.cancel_sftp_transfer(tab_id, transfer_id, cx);
                                    });
                                },
                            ))
                            .into_any_element(),
                    )
                }
                SftpTransferStatus::Done
                | SftpTransferStatus::Cancelled
                | SftpTransferStatus::Failed(_) => {
                    let delete_entity = entity.clone();
                    Some(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .flex_shrink_0()
                            .child(icon_button(
                                AppIcon::Trash,
                                22.0,
                                6.0,
                                Some(roles.surface_container_low),
                                Some(roles.on_surface_variant),
                                None,
                                move |_window, cx| {
                                    delete_entity.update(cx, |this, cx| {
                                        this.remove_sftp_transfer_record(tab_id, transfer_id, cx);
                                    });
                                },
                            ))
                            .into_any_element(),
                    )
                }
            };

            let has_children = !transfer.children.is_empty();
            let expanded = transfer.expanded && has_children;
            let expand_entity = entity.clone();
            let expand_control = if has_children {
                icon_button_with_tooltip(
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
                    20.0,
                    6.0,
                    Some(roles.surface_container_low),
                    Some(roles.on_surface_variant),
                    None,
                    move |_window, cx| {
                        expand_entity.update(cx, |this, cx| {
                            this.toggle_sftp_transfer_expanded(tab_id, transfer_id, cx);
                        });
                    },
                )
            } else {
                div().size(px(20.0))
            };

            let mut child_rows = v_flex().w_full().gap_1().flex_shrink_0();
            if expanded {
                let omitted_child_count = transfer.omitted_child_count();
                if omitted_child_count > 0 {
                    let shown = transfer.children.len().to_string();
                    let total = transfer.child_count.to_string();
                    child_rows = child_rows.child(
                        div()
                            .w_full()
                            .flex_shrink_0()
                            .pl(px(40.0))
                            .pr_2()
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
                    let child_progress = child.bytes_total.map_or_else(
                        || format_byte_size(Some(child.bytes_complete)).to_string(),
                        |total| {
                            format!(
                                "{} / {}",
                                format_byte_size(Some(child.bytes_complete)),
                                format_byte_size(Some(total))
                            )
                        },
                    );
                    let child_progress_value = match child.bytes_total {
                        Some(total) if total > 0 => {
                            ((child.bytes_complete as f32 / total as f32) * 100.0).clamp(0.0, 100.0)
                        }
                        Some(_) if matches!(&child.status, SftpTransferChildStatus::Done) => 100.0,
                        Some(_) => 0.0,
                        None if matches!(&child.status, SftpTransferChildStatus::Done) => 100.0,
                        None => 0.0,
                    };
                    let child_progress_loading = child.bytes_total.is_none()
                        && matches!(&child.status, SftpTransferChildStatus::Running);
                    let (child_status_label, child_accent) = match &child.status {
                        SftpTransferChildStatus::Running => (
                            i18n::string("sftp.transfer_status.running"),
                            extended.info.color,
                        ),
                        SftpTransferChildStatus::Paused => (
                            i18n::string("sftp.transfer_status.paused"),
                            extended.warning.color,
                        ),
                        SftpTransferChildStatus::Done => (
                            i18n::string("sftp.transfer_status.done"),
                            extended.success.color,
                        ),
                        SftpTransferChildStatus::Cancelled => (
                            i18n::string("sftp.transfer_status.cancelled"),
                            roles.on_surface_variant,
                        ),
                        SftpTransferChildStatus::Failed(message) => (
                            i18n::string_args(
                                "sftp.transfer_status.failed",
                                &[("message", message)],
                            ),
                            extended.warning.color,
                        ),
                    };

                    child_rows = child_rows.child(
                        div()
                            .id(SharedString::from(format!(
                                "sftp-transfer-child-{tab_id}-{}-{}",
                                transfer_id.0, child.child_id.0
                            )))
                            .w_full()
                            .flex_shrink_0()
                            .pl(px(40.0))
                            .pr_2()
                            .py_1()
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .size(px(18.0))
                                                    .flex_shrink_0()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .text_color(rgb(child_accent))
                                                    .child(Icon::new(AppIcon::File).small()),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .overflow_hidden()
                                                    .text_size(
                                                        miaominal_settings::FontSize::Body.scaled(),
                                                    )
                                                    .text_color(rgb(roles.on_surface))
                                                    .child(child.relative_path.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_size(
                                                        miaominal_settings::FontSize::Body.scaled(),
                                                    )
                                                    .text_color(rgb(roles.on_surface_variant))
                                                    .child(child_progress),
                                            )
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_size(
                                                        miaominal_settings::FontSize::Body.scaled(),
                                                    )
                                                    .text_color(rgb(child_accent))
                                                    .child(child_status_label),
                                            ),
                                    )
                                    .child(
                                        Progress::new(format!(
                                            "sftp-transfer-child-progress-{tab_id}-{}-{}",
                                            transfer_id.0, child.child_id.0
                                        ))
                                        .with_size(gpui_component::Size::Small)
                                        .value(child_progress_value)
                                        .loading(child_progress_loading)
                                        .color(rgb(child_accent)),
                                    ),
                            ),
                    );
                }
            }

            rows = rows.child(
                div()
                    .id(SharedString::from(format!(
                        "sftp-transfer-row-{}-{}",
                        tab_id, transfer.transfer_id.0
                    )))
                    .w_full()
                    .flex_shrink_0()
                    .px_2()
                    .py_2()
                    .rounded(px(8.0))
                    .when(
                        matches!(
                            &transfer.status,
                            SftpTransferStatus::Running | SftpTransferStatus::Paused
                        ),
                        |this| this.bg(rgb(roles.surface_container_high)),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_3()
                                    .child(expand_control)
                                    .child(
                                        div()
                                            .w(px(20.0))
                                            .flex_shrink_0()
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
                                            .text_size(miaominal_settings::FontSize::Body.scaled())
                                            .text_color(rgb(roles.on_surface))
                                            .child(format!(
                                                "[{}] {} -> {}",
                                                profile_label,
                                                transfer.source.display(),
                                                transfer.destination
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_size(miaominal_settings::FontSize::Body.scaled())
                                            .text_color(rgb(roles.on_surface_variant))
                                            .child(progress),
                                    )
                                    .child(
                                        div()
                                            .text_size(miaominal_settings::FontSize::Body.scaled())
                                            .text_color(rgb(accent))
                                            .child(status_label),
                                    )
                                    .when_some(speed_label, |this, speed| {
                                        this.child(
                                            div()
                                                .text_size(
                                                    miaominal_settings::FontSize::Body.scaled(),
                                                )
                                                .text_color(rgb(text_muted))
                                                .child(speed),
                                        )
                                    }),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div().flex_1().child(
                                            Progress::new(format!(
                                                "sftp-transfer-progress-{tab_id}-{}",
                                                transfer_id.0
                                            ))
                                            .with_size(gpui_component::Size::Small)
                                            .value(progress_value)
                                            .loading(progress_loading)
                                            .color(rgb(accent)),
                                        ),
                                    )
                                    .when_some(transfer_actions, |this, actions| {
                                        this.child(actions)
                                    }),
                            )
                            .when(expanded, |this| this.child(child_rows)),
                    ),
            );
        }

        if !has_transfers {
            return sftp_progress_center_card(section_id, sftp_empty_transfer_summary(first_error))
                .into_any_element();
        }

        sftp_progress_center_card(
            section_id,
            div().size_full().overflow_y_scrollbar().p_2().child(rows),
        )
        .into_any_element()
    }

    fn cache_sftp_browser_container_width(
        &mut self,
        tab_id: usize,
        width: Pixels,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.sftp_tab_mut(tab_id) else {
            return;
        };

        if tab.layout.browser_container_width != width {
            tab.layout.browser_container_width = width;
            cx.notify();
        }
    }

    fn cache_sftp_page_container_height(
        &mut self,
        tab_id: usize,
        height: Pixels,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.sftp_tab_mut(tab_id) else {
            return;
        };

        if tab.layout.page_container_height != height {
            tab.layout.page_container_height = height;
            cx.notify();
        }
    }

    fn start_sftp_split_drag(
        &mut self,
        tab_id: usize,
        divider: SftpSplitDivider,
        initial_pointer: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.sftp_tab_mut(tab_id) else {
            return;
        };

        let (initial_flex_a, container_size) = match divider {
            SftpSplitDivider::BrowserPanels => {
                let flex_a = sftp_local_panel_flex(tab);
                (
                    flex_a,
                    sftp_usable_container_size(tab.layout.browser_container_width),
                )
            }
            SftpSplitDivider::ProgressCenter => {
                let flex_a = sftp_browser_area_flex(tab);
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
        cx.notify();
    }

    fn update_sftp_split_drag(&mut self, tab_id: usize, pointer: f32, cx: &mut Context<Self>) {
        let Some(tab) = self.sftp_tab_mut(tab_id) else {
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
                    cx.notify();
                }
            }
        }
    }

    fn finish_sftp_split_drag(&mut self, tab_id: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.sftp_tab_mut(tab_id) else {
            return;
        };

        if tab.layout.drag.take().is_some() {
            cx.notify();
        }
    }

    fn finish_sftp_page_pointer_drag(
        &mut self,
        tab_id: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let is_split_dragging = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .is_some_and(|tab| tab.layout.drag.is_some());

        if is_split_dragging {
            self.finish_sftp_split_drag(tab_id, cx);
            return true;
        }

        self.finish_active_sftp_drag_selection(tab_id, position, cx)
    }

    pub(in crate::ui::shell) fn render_sftp_page_for_tab(
        &mut self,
        entity: Entity<Self>,
        tab_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.render_sftp_page_content(entity, tab_id, window, cx)
    }

    pub(in crate::ui::shell) fn render_sftp_remote_browser_panel(
        &self,
        entity: Entity<Self>,
        tab_id: usize,
        sftp_tab: &SftpTabState,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let extended = material.extended;

        let remote_path_bar = if self.workspace_forms.sftp_browser.remote_path_editing {
            let up_entity = entity.clone();
            sftp_path_bar(
                sftp_path_input_shell(&self.workspace_forms.sftp_browser.remote_path_input),
                false,
                move |_window, cx| {
                    up_entity.update(cx, |this, cx| {
                        this.navigate_sftp_remote_up(tab_id, cx);
                    });
                },
                |_window, _cx| {},
            )
            .into_any_element()
        } else {
            let breadcrumb_entity = entity.clone();
            let up_entity = entity.clone();
            let edit_entity = entity.clone();
            sftp_path_bar(
                sftp_path_breadcrumb_shell(
                    SharedString::from(format!("session-remote-sftp-path-{tab_id}")),
                    build_remote_sftp_breadcrumb(&sftp_tab.remote_path, breadcrumb_entity, tab_id),
                    sftp_tab.remote_path.clone(),
                ),
                true,
                move |_window, cx| {
                    up_entity.update(cx, |this, cx| {
                        this.navigate_sftp_remote_up(tab_id, cx);
                    });
                },
                move |_window, cx| {
                    edit_entity.update(cx, |this, cx| {
                        this.set_sftp_remote_path_editing(true, cx);
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
                    let entity = entity.clone();
                    move |_window, cx| {
                        entity.update(cx, |this, cx| {
                            let path = this
                                .workspace_state
                                .tabs
                                .iter()
                                .find(|tab| tab.id == tab_id)
                                .and_then(TabState::as_sftp)
                                .map(|sftp| sftp.remote_path.clone())
                                .unwrap_or_else(|| ".".into());
                            this.request_sftp_remote_directory(tab_id, path, cx);
                        });
                    }
                },
            ))
            .child(sftp_toolbar_button(
                AppIcon::Plus,
                i18n::string("sftp.tooltips.create_directory"),
                {
                    let entity = entity.clone();
                    move |window, cx| {
                        entity.update(cx, |this, cx| {
                            this.begin_sftp_create_directory(tab_id, window, cx);
                        });
                    }
                },
            ))
            .child(sftp_toolbar_button(
                AppIcon::Download,
                i18n::string("sftp.tooltips.download_selected"),
                {
                    let entity = entity.clone();
                    move |_window, cx| {
                        entity.update(cx, |this, cx| {
                            this.queue_sftp_download_selected(tab_id, cx);
                        });
                    }
                },
            ))
            .into_any_element();

        let table_row_height = gpui_component::Size::Small.table_row_height();
        let remote_table_bounds = Rc::new(RefCell::new(None));
        let remote_sftp_table = self.workspace_forms.sftp_browser.remote_table.clone();
        let remote_table_for_menu = self.workspace_forms.sftp_browser.remote_table.clone();
        let remote_selected_count = sftp_tab.selected_remote_paths.len();

        let remote_list = div()
            .id(("sftp-remote-table-wrap", tab_id))
            .group("sftp-remote-drop")
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .on_prepaint({
                let remote_table_bounds = remote_table_bounds.clone();
                let table = self.workspace_forms.sftp_browser.remote_table.clone();
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
                let entity = entity.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event: &MouseDownEvent, _window, cx| {
                    if cx.has_active_drag() {
                        return;
                    }
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    entity.update(cx, |this, _cx| {
                        this.begin_sftp_drag_selection(
                            tab_id,
                            SftpBrowserSide::Remote,
                            event.position,
                            bounds,
                            table_row_height,
                            _cx,
                        );
                    });
                }
            })
            .on_mouse_move({
                let entity = entity.clone();
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

                    entity.update(cx, |this, cx| {
                        if this.update_sftp_drag_selection(
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
                let entity = entity.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event: &MouseUpEvent, _window, cx| {
                    if event.button != MouseButton::Left {
                        return;
                    }
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    entity.update(cx, |this, cx| {
                        if this.finish_sftp_drag_selection(
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
                let entity = entity.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event, _window, cx| {
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    entity.update(cx, |this, cx| {
                        this.handle_sftp_blank_click(
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
                DataTable::new(&self.workspace_forms.sftp_browser.remote_table)
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
                let entity = entity.clone();
                move |paths: &ExternalPaths, _window, cx| {
                    let local_paths: Vec<PathBuf> = paths.paths().to_vec();
                    entity.update(cx, |this, cx| {
                        this.queue_sftp_upload_paths(tab_id, local_paths, cx);
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
                let entity = entity.clone();
                let remote_table_for_menu = remote_table_for_menu.clone();
                move |menu, _window: &mut Window, cx: &mut Context<PopupMenu>| {
                    let is_header = remote_table_for_menu.update(cx, |state, _| {
                        state.delegate_mut().take_col_header_right_clicked()
                    });
                    if is_header {
                        return menu;
                    }
                    build_remote_sftp_context_menu(menu, entity.clone(), tab_id, cx)
                }
            },
        })
        .into_any_element()
    }

    pub(in crate::ui::shell) fn render_sftp_page_content(
        &mut self,
        entity: Entity<Self>,
        tab_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let extended = material.extended;
        let progress_center_visibility =
            self.sftp_progress_center_render_visibility(tab_id, window);
        let Some(tab) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
        else {
            return self.render_snippets_page_content();
        };
        let Some(sftp_tab) = tab.as_sftp() else {
            return self.render_snippets_page_content();
        };

        let local_panel_flex = sftp_local_panel_flex(sftp_tab);
        let browser_area_flex = sftp_browser_area_flex(sftp_tab);
        let progress_center_visibility = progress_center_visibility.unwrap_or(0.0);
        let progress_center_visible = progress_center_visibility > 0.0;
        let progress_center_footer_flex = (1.0 - browser_area_flex) * progress_center_visibility;
        let browser_panel_flex = 1.0 - progress_center_footer_flex;
        let local_path_bar = if self.workspace_forms.sftp_browser.local_path_editing {
            let up_entity = entity.clone();
            sftp_path_bar(
                sftp_path_input_shell(&self.workspace_forms.sftp_browser.local_path_input),
                false,
                move |_window, cx| {
                    up_entity.update(cx, |this, cx| {
                        this.navigate_sftp_local_up(tab_id, cx);
                    });
                },
                |_window, _cx| {},
            )
            .into_any_element()
        } else {
            let breadcrumb_entity = entity.clone();
            let up_entity = entity.clone();
            let edit_entity = entity.clone();
            sftp_path_bar(
                sftp_path_breadcrumb_shell(
                    SharedString::from(format!("local-sftp-path-{tab_id}")),
                    build_local_sftp_breadcrumb(&sftp_tab.local_path, breadcrumb_entity, tab_id),
                    AppView::display_sftp_local_path(&sftp_tab.local_path),
                ),
                true,
                move |_window, cx| {
                    up_entity.update(cx, |this, cx| {
                        this.navigate_sftp_local_up(tab_id, cx);
                    });
                },
                move |_window, cx| {
                    edit_entity.update(cx, |this, cx| {
                        this.set_sftp_local_path_editing(true, cx);
                    });
                },
            )
            .into_any_element()
        };
        let remote_path_bar = if self.workspace_forms.sftp_browser.remote_path_editing {
            let up_entity = entity.clone();
            sftp_path_bar(
                sftp_path_input_shell(&self.workspace_forms.sftp_browser.remote_path_input),
                false,
                move |_window, cx| {
                    up_entity.update(cx, |this, cx| {
                        this.navigate_sftp_remote_up(tab_id, cx);
                    });
                },
                |_window, _cx| {},
            )
            .into_any_element()
        } else {
            let breadcrumb_entity = entity.clone();
            let up_entity = entity.clone();
            let edit_entity = entity.clone();
            sftp_path_bar(
                sftp_path_breadcrumb_shell(
                    SharedString::from(format!("remote-sftp-path-{tab_id}")),
                    build_remote_sftp_breadcrumb(&sftp_tab.remote_path, breadcrumb_entity, tab_id),
                    sftp_tab.remote_path.clone(),
                ),
                true,
                move |_window, cx| {
                    up_entity.update(cx, |this, cx| {
                        this.navigate_sftp_remote_up(tab_id, cx);
                    });
                },
                move |_window, cx| {
                    edit_entity.update(cx, |this, cx| {
                        this.set_sftp_remote_path_editing(true, cx);
                    });
                },
            )
            .into_any_element()
        };

        let table_row_height = gpui_component::Size::Small.table_row_height();
        let local_table_bounds = Rc::new(RefCell::new(None));
        let remote_table_bounds = Rc::new(RefCell::new(None));
        let local_sftp_table = self.workspace_forms.sftp_browser.local_table.clone();
        let remote_sftp_table = self.workspace_forms.sftp_browser.remote_table.clone();
        let local_table_for_menu = self.workspace_forms.sftp_browser.local_table.clone();
        let remote_table_for_menu = self.workspace_forms.sftp_browser.remote_table.clone();
        let local_selected_count = sftp_tab.selected_local_paths.len();
        let remote_selected_count = sftp_tab.selected_remote_paths.len();

        let local_toolbar = h_flex()
            .items_center()
            .gap(px(SFTP_ACTION_BUTTON_GAP))
            .child(sftp_toolbar_button(
                AppIcon::Rotate,
                i18n::string("sftp.tooltips.refresh_local"),
                {
                    let entity = entity.clone();
                    move |_window, cx| {
                        entity.update(cx, |this, cx| {
                            this.refresh_sftp_local_directory(tab_id, cx);
                        });
                    }
                },
            ))
            .child(sftp_toolbar_button(
                AppIcon::Upload,
                i18n::string("sftp.tooltips.upload_selected"),
                {
                    let entity = entity.clone();
                    move |_window, cx| {
                        entity.update(cx, |this, cx| {
                            this.queue_sftp_upload_selected(tab_id, cx);
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
                    let entity = entity.clone();
                    move |_window, cx| {
                        entity.update(cx, |this, cx| {
                            let path = this
                                .workspace_state
                                .tabs
                                .iter()
                                .find(|tab| tab.id == tab_id)
                                .and_then(TabState::as_sftp)
                                .map(|sftp| sftp.remote_path.clone())
                                .unwrap_or_else(|| ".".into());
                            this.request_sftp_remote_directory(tab_id, path, cx);
                        });
                    }
                },
            ))
            .child(sftp_toolbar_button(
                AppIcon::Plus,
                i18n::string("sftp.tooltips.create_directory"),
                {
                    let entity = entity.clone();
                    move |window, cx| {
                        entity.update(cx, |this, cx| {
                            this.begin_sftp_create_directory(tab_id, window, cx);
                        });
                    }
                },
            ))
            .child(sftp_toolbar_button(
                AppIcon::Download,
                i18n::string("sftp.tooltips.download_selected"),
                {
                    let entity = entity.clone();
                    move |_window, cx| {
                        entity.update(cx, |this, cx| {
                            this.queue_sftp_download_selected(tab_id, cx);
                        });
                    }
                },
            ))
            .into_any_element();

        let local_list = div()
            .id(("sftp-local-table-wrap", tab_id))
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .on_prepaint({
                let local_table_bounds = local_table_bounds.clone();
                let table = self.workspace_forms.sftp_browser.local_table.clone();
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
                let entity = entity.clone();
                let local_table_bounds = local_table_bounds.clone();
                move |event: &MouseDownEvent, _window, cx| {
                    if cx.has_active_drag() {
                        return;
                    }
                    let Some(bounds) = *local_table_bounds.borrow() else {
                        return;
                    };

                    entity.update(cx, |this, _cx| {
                        this.begin_sftp_drag_selection(
                            tab_id,
                            SftpBrowserSide::Local,
                            event.position,
                            bounds,
                            table_row_height,
                            _cx,
                        );
                    });
                }
            })
            .on_mouse_move({
                let entity = entity.clone();
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

                    entity.update(cx, |this, cx| {
                        if this.update_sftp_drag_selection(
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
                let entity = entity.clone();
                let local_table_bounds = local_table_bounds.clone();
                move |event: &MouseUpEvent, _window, cx| {
                    if event.button != MouseButton::Left {
                        return;
                    }
                    let Some(bounds) = *local_table_bounds.borrow() else {
                        return;
                    };

                    entity.update(cx, |this, cx| {
                        if this.finish_sftp_drag_selection(
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
                let entity = entity.clone();
                let local_table_bounds = local_table_bounds.clone();
                move |event, _window, cx| {
                    let Some(bounds) = *local_table_bounds.borrow() else {
                        return;
                    };

                    entity.update(cx, |this, cx| {
                        this.handle_sftp_blank_click(
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
                DataTable::new(&self.workspace_forms.sftp_browser.local_table)
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
            .id(("sftp-remote-table-wrap", tab_id))
            .group("sftp-remote-drop")
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .on_prepaint({
                let remote_table_bounds = remote_table_bounds.clone();
                let table = self.workspace_forms.sftp_browser.remote_table.clone();
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
                let entity = entity.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event: &MouseDownEvent, _window, cx| {
                    if cx.has_active_drag() {
                        return;
                    }
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    entity.update(cx, |this, _cx| {
                        this.begin_sftp_drag_selection(
                            tab_id,
                            SftpBrowserSide::Remote,
                            event.position,
                            bounds,
                            table_row_height,
                            _cx,
                        );
                    });
                }
            })
            .on_mouse_move({
                let entity = entity.clone();
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

                    entity.update(cx, |this, cx| {
                        if this.update_sftp_drag_selection(
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
                let entity = entity.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event: &MouseUpEvent, _window, cx| {
                    if event.button != MouseButton::Left {
                        return;
                    }
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    entity.update(cx, |this, cx| {
                        if this.finish_sftp_drag_selection(
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
                let entity = entity.clone();
                let remote_table_bounds = remote_table_bounds.clone();
                move |event, _window, cx| {
                    let Some(bounds) = *remote_table_bounds.borrow() else {
                        return;
                    };

                    entity.update(cx, |this, cx| {
                        this.handle_sftp_blank_click(
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
                DataTable::new(&self.workspace_forms.sftp_browser.remote_table)
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
                let entity = entity.clone();
                move |paths: &ExternalPaths, _window, cx| {
                    let local_paths: Vec<PathBuf> = paths.paths().to_vec();
                    entity.update(cx, |this, cx| {
                        this.queue_sftp_upload_paths(tab_id, local_paths, cx);
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

        let footer = self
            .render_sftp_progress_center(entity.clone(), format!("sftp-progress-center-{tab_id}"));

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
                            let entity = entity.clone();
                            let local_table_for_menu = local_table_for_menu.clone();
                            move |menu, _window: &mut Window, cx: &mut Context<PopupMenu>| {
                                let is_header = local_table_for_menu.update(cx, |state, _| {
                                    state.delegate_mut().take_col_header_right_clicked()
                                });
                                if is_header {
                                    return menu;
                                }
                                build_local_sftp_context_menu(menu, entity.clone(), tab_id, cx)
                            }
                        },
                    })),
            )
            .child(sftp_split_bar(
                tab_id,
                SftpSplitDivider::BrowserPanels,
                matches!(
                    sftp_tab.layout.drag.as_ref(),
                    Some(drag) if drag.divider == SftpSplitDivider::BrowserPanels
                ),
                cx,
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
                            let entity = entity.clone();
                            let remote_table_for_menu = remote_table_for_menu.clone();
                            move |menu, _window: &mut Window, cx: &mut Context<PopupMenu>| {
                                let is_header = remote_table_for_menu.update(cx, |state, _| {
                                    state.delegate_mut().take_col_header_right_clicked()
                                });
                                if is_header {
                                    return menu;
                                }
                                build_remote_sftp_context_menu(menu, entity.clone(), tab_id, cx)
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
                        let _ = this.finish_active_sftp_drag_selection(tab_id, event.position, cx);
                    }
                },
            ))
            .on_mouse_move(
                cx.listener(move |this, event: &MouseMoveEvent, _window, cx| {
                    if event.pressed_button != Some(MouseButton::Left) {
                        if this.finish_active_sftp_drag_selection(tab_id, event.position, cx) {
                            cx.stop_propagation();
                        }
                        return;
                    }

                    if let Some(divider) = this
                        .workspace_state
                        .tabs
                        .iter()
                        .find(|tab| tab.id == tab_id)
                        .and_then(TabState::as_sftp)
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

                    if this.update_active_sftp_drag_selection(tab_id, event.position, cx) {
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
                                            tab_id,
                                            SftpSplitDivider::ProgressCenter,
                                            matches!(
                                                sftp_tab.layout.drag.as_ref(),
                                                Some(drag) if drag.divider == SftpSplitDivider::ProgressCenter
                                            ),
                                            cx,
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
        tab_id: usize,
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
                HintedInput::new(&self.workspace_forms.sftp_browser.prompt_input)
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
            let entity = entity.clone();
            move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.cancel_sftp_prompt(cx);
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
            let entity = entity.clone();
            move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.commit_sftp_prompt(cx);
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
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.skip_sftp_overwrite_prompt(cx);
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
