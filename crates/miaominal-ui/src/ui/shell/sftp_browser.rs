use super::*;
use crate::ui::i18n;
use gpui_component::ElementExt;
use gpui_component::table::ColumnSort;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SftpBrowserSide {
    Local,
    Remote,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct SftpBrowserTableRow {
    pub(in crate::ui::shell) path: String,
    pub(in crate::ui::shell) name: SharedString,
    pub(in crate::ui::shell) is_directory: bool,
    pub(in crate::ui::shell) kind: miaominal_sftp::SftpEntryKind,
    pub(in crate::ui::shell) size: Option<u64>,
    pub(in crate::ui::shell) type_label: SharedString,
    pub(in crate::ui::shell) modified: Option<SystemTime>,
    pub(in crate::ui::shell) attributes: Option<SharedString>,
    pub(in crate::ui::shell) owner: Option<SharedString>,
    pub(in crate::ui::shell) depth: usize,
    pub(in crate::ui::shell) is_expanded: bool,
    pub(in crate::ui::shell) is_loading_children: bool,
}

impl SftpBrowserTableRow {
    pub(in crate::ui::shell) fn from_local(entry: &LocalSftpEntry) -> Self {
        let kind = if entry.is_directory {
            miaominal_sftp::SftpEntryKind::Directory
        } else {
            miaominal_sftp::SftpEntryKind::File
        };

        Self {
            path: entry.path.display().to_string(),
            name: entry.filename.clone().into(),
            is_directory: entry.is_directory,
            kind,
            size: entry.size,
            type_label: Self::localized_type_label(SftpBrowserSide::Local, kind).into(),
            modified: entry.modified,
            attributes: entry.attributes.clone().map(Into::into),
            owner: entry.owner.clone().map(Into::into),
            depth: 0,
            is_expanded: false,
            is_loading_children: false,
        }
    }

    pub(in crate::ui::shell) fn from_remote(entry: &SftpEntry) -> Self {
        let type_label: SharedString =
            Self::localized_type_label(SftpBrowserSide::Remote, entry.kind).into();

        Self {
            path: entry.path.clone(),
            name: entry.filename.clone().into(),
            is_directory: entry.kind == miaominal_sftp::SftpEntryKind::Directory,
            kind: entry.kind,
            size: entry.size,
            type_label,
            modified: entry.modified,
            attributes: entry.attributes.clone().map(Into::into),
            owner: entry.owner.clone().map(Into::into),
            depth: 0,
            is_expanded: false,
            is_loading_children: false,
        }
    }

    fn refresh_localized_text(&mut self, side: SftpBrowserSide) {
        self.type_label = Self::localized_type_label(side, self.kind).into();
    }

    fn localized_type_label(side: SftpBrowserSide, kind: miaominal_sftp::SftpEntryKind) -> String {
        match (side, kind) {
            (SftpBrowserSide::Local, miaominal_sftp::SftpEntryKind::Directory) => {
                i18n::string("sftp_browser.types.folder")
            }
            (_, miaominal_sftp::SftpEntryKind::Directory) => {
                i18n::string("sftp_browser.types.directory")
            }
            (_, miaominal_sftp::SftpEntryKind::Symlink) => {
                i18n::string("sftp_browser.types.symlink")
            }
            (_, miaominal_sftp::SftpEntryKind::Other) => i18n::string("sftp_browser.types.other"),
            (_, miaominal_sftp::SftpEntryKind::File) => i18n::string("sftp_browser.types.file"),
        }
    }
}

pub(in crate::ui::shell) struct SftpBrowserTableDelegate {
    side: SftpBrowserSide,
    app_view: WeakEntity<AppView>,
    tab_id: usize,
    source_rows: Vec<SftpBrowserTableRow>,
    children_source: HashMap<String, Vec<SftpBrowserTableRow>>,
    expanded_paths: HashSet<String>,
    loading_paths: HashSet<String>,
    rows: Vec<SftpBrowserTableRow>,
    selected_row: Option<usize>,
    selected_paths: HashSet<String>,
    primary_selected_path: Option<String>,
    row_bounds: Vec<Bounds<Pixels>>,
    empty_message: SharedString,
    loading: bool,
    sort_state: Option<(usize, ColumnSort)>,
    available_width: Pixels,
    hidden_columns: HashSet<usize>,
    visible_col_map: Vec<usize>,
    pub(in crate::ui::shell) col_header_right_clicked: bool,
    pub(in crate::ui::shell) inline_rename_path: Option<String>,
}

impl SftpBrowserTableDelegate {
    pub(in crate::ui::shell) fn new(side: SftpBrowserSide, app_view: WeakEntity<AppView>) -> Self {
        let hidden_columns = HashSet::from([4usize, 5usize]);
        let visible_col_map = (0..6usize)
            .filter(|i| !hidden_columns.contains(i))
            .collect();

        Self {
            side,
            app_view,
            tab_id: 0,
            source_rows: Vec::new(),
            children_source: HashMap::new(),
            expanded_paths: HashSet::new(),
            loading_paths: HashSet::new(),
            rows: Vec::new(),
            selected_row: None,
            selected_paths: HashSet::new(),
            primary_selected_path: None,
            row_bounds: Vec::new(),
            empty_message: Self::localized_empty_message(side).into(),
            loading: false,
            sort_state: None,
            available_width: px(0.),
            hidden_columns,
            visible_col_map,
            col_header_right_clicked: false,
            inline_rename_path: None,
        }
    }

    pub(in crate::ui::shell) fn set_rows(
        &mut self,
        rows: Vec<SftpBrowserTableRow>,
        loading: bool,
        tab_id: usize,
    ) {
        self.source_rows = rows;
        self.loading = loading;
        self.empty_message = Self::localized_empty_message(self.side).into();
        self.tab_id = tab_id;
        self.children_source.clear();
        self.expanded_paths.clear();
        self.loading_paths.clear();
        self.apply_sort();
    }

    pub(in crate::ui::shell) fn row(&self, row_ix: usize) -> Option<&SftpBrowserTableRow> {
        self.rows.get(row_ix)
    }

    pub(in crate::ui::shell) fn row_index_by_path(&self, path: &str) -> Option<usize> {
        self.rows.iter().position(|row| row.path == path)
    }

    pub(in crate::ui::shell) fn set_selected_paths(
        &mut self,
        paths: Vec<String>,
        primary_path: Option<String>,
    ) {
        let mut first_path = None;
        let mut selected_paths = HashSet::new();

        for path in paths {
            if first_path.is_none() {
                first_path = Some(path.clone());
            }
            selected_paths.insert(path);
        }

        self.primary_selected_path = primary_path.or(first_path);
        self.selected_paths = selected_paths;
        self.selected_row = self
            .primary_selected_path
            .as_deref()
            .and_then(|path| self.row_index_by_path(path));
    }

    pub(in crate::ui::shell) fn paths_in_bounds(
        &self,
        selection_bounds: Bounds<Pixels>,
    ) -> Vec<String> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(row_ix, row)| {
                let row_bounds = self.row_bounds.get(row_ix)?;
                Self::bounds_intersect(*row_bounds, selection_bounds).then(|| row.path.clone())
            })
            .collect()
    }

    pub(in crate::ui::shell) fn receive_children(
        &mut self,
        path: String,
        children: Vec<SftpBrowserTableRow>,
        cx: &mut Context<TableState<Self>>,
    ) {
        self.loading_paths.remove(&path);
        self.children_source.insert(path.clone(), children);
        self.expanded_paths.insert(path);
        self.apply_sort();
        cx.notify();
    }

    pub(in crate::ui::shell) fn refresh_localized_text(&mut self) {
        self.empty_message = Self::localized_empty_message(self.side).into();
        for row in &mut self.source_rows {
            row.refresh_localized_text(self.side);
        }
        for children in self.children_source.values_mut() {
            for row in children {
                row.refresh_localized_text(self.side);
            }
        }
        self.apply_sort();
    }

    fn localized_empty_message(side: SftpBrowserSide) -> String {
        match side {
            SftpBrowserSide::Local => i18n::string("sftp_browser.empty.local"),
            SftpBrowserSide::Remote => i18n::string("sftp_browser.empty.remote"),
        }
    }

    fn column_label(col_ix: usize) -> String {
        match col_ix {
            0 => i18n::string("sftp_browser.columns.name"),
            1 => i18n::string("sftp_browser.columns.size"),
            2 => i18n::string("sftp_browser.columns.type"),
            3 => i18n::string("sftp_browser.columns.modified"),
            4 => i18n::string("sftp_browser.columns.attributes"),
            _ => i18n::string("sftp_browser.columns.owner"),
        }
    }

    fn apply_sort(&mut self) {
        self.rows = Vec::new();
        let source = self.source_rows.clone();
        self.flatten_into_visible(&source, 0);
        self.row_bounds = vec![Bounds::default(); self.rows.len()];
        self.selected_row = self
            .primary_selected_path
            .as_deref()
            .and_then(|path| self.row_index_by_path(path));
    }

    fn flatten_into_visible(&mut self, entries: &[SftpBrowserTableRow], depth: usize) {
        let mut sorted = entries.to_vec();
        if let Some((col_ix, sort)) = self.sort_state
            && !matches!(sort, ColumnSort::Default)
        {
            sorted.sort_by(|left, right| Self::compare_rows(left, right, col_ix, sort));
        } else {
            sorted.sort_by(|left, right| {
                right
                    .is_directory
                    .cmp(&left.is_directory)
                    .then_with(|| Self::compare_text(left.name.as_ref(), right.name.as_ref()))
            });
        }

        for mut row in sorted {
            row.depth = depth;
            row.is_expanded = self.expanded_paths.contains(&row.path);
            row.is_loading_children = self.loading_paths.contains(&row.path);
            let path = row.path.clone();
            let is_directory = row.is_directory;
            let is_expanded = row.is_expanded;
            self.rows.push(row);
            if is_directory
                && is_expanded
                && let Some(children) = self.children_source.get(&path).cloned()
            {
                self.flatten_into_visible(&children, depth + 1);
            }
        }
    }

    fn compare_rows(
        left: &SftpBrowserTableRow,
        right: &SftpBrowserTableRow,
        col_ix: usize,
        sort: ColumnSort,
    ) -> Ordering {
        let directory_order = right.is_directory.cmp(&left.is_directory);
        if directory_order != Ordering::Equal {
            return directory_order;
        }

        let order = match col_ix {
            0 => Self::compare_text(left.name.as_ref(), right.name.as_ref()),
            1 => left.size.cmp(&right.size),
            2 => Self::compare_text(left.type_label.as_ref(), right.type_label.as_ref()),
            3 => left.modified.cmp(&right.modified),
            4 => {
                Self::compare_optional_text(left.attributes.as_deref(), right.attributes.as_deref())
            }
            _ => Self::compare_optional_text(left.owner.as_deref(), right.owner.as_deref()),
        };

        let order = match sort {
            ColumnSort::Ascending => order,
            ColumnSort::Descending => order.reverse(),
            ColumnSort::Default => Ordering::Equal,
        };

        if order == Ordering::Equal {
            Self::compare_text(left.path.as_str(), right.path.as_str())
        } else {
            order
        }
    }

    fn compare_text(left: &str, right: &str) -> Ordering {
        left.to_lowercase().cmp(&right.to_lowercase())
    }

    fn compare_optional_text(left: Option<&str>, right: Option<&str>) -> Ordering {
        match (left, right) {
            (Some(left), Some(right)) => Self::compare_text(left, right),
            (Some(_), None) => Ordering::Greater,
            (None, Some(_)) => Ordering::Less,
            (None, None) => Ordering::Equal,
        }
    }

    fn bounds_intersect(left: Bounds<Pixels>, right: Bounds<Pixels>) -> bool {
        left.left() <= right.right()
            && left.right() >= right.left()
            && left.top() <= right.bottom()
            && left.bottom() >= right.top()
    }

    fn collapse_directory(&mut self, path: String) {
        self.expanded_paths.remove(&path);
        self.apply_sort();
    }

    fn expand_cached_directory(&mut self, path: String) {
        self.expanded_paths.insert(path);
        self.apply_sort();
    }

    pub(in crate::ui::shell) fn set_available_width(&mut self, width: Pixels) -> bool {
        let diff = ((self.available_width - width) / px(1.0)).abs();
        if diff > 1.0 {
            self.available_width = width;
            true
        } else {
            false
        }
    }

    pub(in crate::ui::shell) fn cancel_expand(&mut self, path: &str) {
        self.loading_paths.remove(path);
        self.apply_sort();
    }

    pub(in crate::ui::shell) fn take_col_header_right_clicked(&mut self) -> bool {
        let v = self.col_header_right_clicked;
        self.col_header_right_clicked = false;
        v
    }

    fn compute_visible_col_map(&mut self) {
        self.visible_col_map = (0..6usize)
            .filter(|i| !self.hidden_columns.contains(i))
            .collect();
    }

    pub(in crate::ui::shell) fn set_hidden_columns(&mut self, hidden_columns: Vec<usize>) {
        self.hidden_columns = hidden_columns.into_iter().collect();
        if self.hidden_columns.len() >= 6 {
            self.hidden_columns = HashSet::from([4usize, 5usize]);
        }
        self.compute_visible_col_map();
        self.sort_state = self
            .sort_state
            .filter(|(sorted_ix, _)| !self.hidden_columns.contains(sorted_ix));
    }

    pub(in crate::ui::shell) fn hidden_columns(&self) -> Vec<usize> {
        let mut columns: Vec<usize> = self.hidden_columns.iter().copied().collect();
        columns.sort_unstable();
        columns
    }

    pub(in crate::ui::shell) fn toggle_column_visibility(&mut self, orig_col_ix: usize) {
        if self.hidden_columns.contains(&orig_col_ix) {
            self.hidden_columns.remove(&orig_col_ix);
        } else if self.visible_col_map.len() > 1 {
            self.hidden_columns.insert(orig_col_ix);
        }
        self.compute_visible_col_map();
        self.sort_state = self
            .sort_state
            .filter(|(sorted_ix, _)| !self.hidden_columns.contains(sorted_ix));
    }
}

impl TableDelegate for SftpBrowserTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.visible_col_map.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let orig_col_ix = self.visible_col_map.get(col_ix).copied().unwrap_or(col_ix);
        let sort = self
            .sort_state
            .and_then(|(sorted_orig_ix, sort)| (sorted_orig_ix == orig_col_ix).then_some(sort));

        const BASE_WIDTHS: [f32; 6] = [240.0, 96.0, 96.0, 124.0, 120.0, 108.0];
        let total_base: f32 = BASE_WIDTHS
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.hidden_columns.contains(i))
            .map(|(_, w)| w)
            .sum();
        let base = px(BASE_WIDTHS[orig_col_ix]);
        let width = if self.available_width > px(0.) {
            let usable = (self.available_width - px(20.0)).max(px(0.0));
            if usable < px(total_base) {
                base * (usable / px(total_base))
            } else {
                base
            }
        } else {
            base
        };

        match orig_col_ix {
            0 => Column::new("name", Self::column_label(0))
                .sort(sort.unwrap_or(ColumnSort::Default))
                .width(width)
                .fixed_left()
                .resizable(true),
            1 => Column::new("size", Self::column_label(1))
                .sort(sort.unwrap_or(ColumnSort::Default))
                .width(width)
                .text_right()
                .resizable(true),
            2 => Column::new("type", Self::column_label(2))
                .sort(sort.unwrap_or(ColumnSort::Default))
                .width(width)
                .resizable(true),
            3 => Column::new("modified", Self::column_label(3))
                .sort(sort.unwrap_or(ColumnSort::Default))
                .width(width)
                .resizable(true),
            4 => Column::new("attributes", Self::column_label(4))
                .sort(sort.unwrap_or(ColumnSort::Default))
                .width(width)
                .resizable(true),
            _ => Column::new("owner", Self::column_label(5))
                .sort(sort.unwrap_or(ColumnSort::Default))
                .width(width)
                .resizable(true),
        }
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        let orig_col_ix = self.visible_col_map.get(col_ix).copied().unwrap_or(col_ix);
        self.sort_state = Some((orig_col_ix, sort));
        self.apply_sort();
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let orig_col_ix = self.visible_col_map.get(col_ix).copied().unwrap_or(col_ix);
        let col_name: SharedString = Self::column_label(orig_col_ix).into();
        let hidden_columns = self.hidden_columns.clone();
        let table = cx.entity().clone();
        let app_view = self.app_view.clone();
        let side = self.side;

        div()
            .id(("sftp-th-menu", col_ix))
            .size_full()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|table, _: &MouseDownEvent, _, cx| {
                    table.set_right_clicked_row(None, cx);
                    table.delegate_mut().col_header_right_clicked = true;
                }),
            )
            .child(col_name)
            .context_menu(move |menu, _window, _cx| {
                (0..6).fold(menu, |menu, i| {
                    let is_visible = !hidden_columns.contains(&i);
                    let table = table.clone();
                    let app_view = app_view.clone();
                    let name = Self::column_label(i);
                    menu.item(PopupMenuItem::new(name).checked(is_visible).on_click(
                        move |_, _, cx| {
                            table.update(cx, |table, cx| {
                                let hidden_columns = {
                                    let delegate = table.delegate_mut();
                                    delegate.toggle_column_visibility(i);
                                    delegate.hidden_columns()
                                };
                                if let Some(app_view) = app_view.upgrade() {
                                    app_view.update(cx, |view, cx| {
                                        view.persist_sftp_browser_hidden_columns(
                                            side,
                                            hidden_columns,
                                            cx,
                                        );
                                    });
                                }
                                table.refresh(cx);
                            });
                        },
                    ))
                })
            })
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let material = miaominal_settings::current_theme().material;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(miaominal_settings::FontSize::Body.scaled())
            .text_color(rgb(text_muted))
            .child(self.empty_message.clone())
            .into_any_element()
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let roles = miaominal_settings::current_theme().material.roles;
        let row_path = self
            .rows
            .get(row_ix)
            .map(|row| row.path.clone())
            .unwrap_or_default();
        let is_selected = self.selected_paths.contains(&row_path);
        let select_path = row_path.clone();
        let context_path = row_path.clone();
        let table_entity = cx.entity().clone();

        div()
            .id(("sftp-row", row_ix))
            .when(is_selected, |this| {
                this.relative()
                    .text_color(rgb(roles.on_primary))
                    .child(div().absolute().inset_0().bg(rgb(roles.primary)))
            })
            .on_prepaint(move |bounds, _, cx| {
                table_entity.update(cx, |table, _| {
                    if let Some(row_bounds) = table.delegate_mut().row_bounds.get_mut(row_ix) {
                        *row_bounds = bounds;
                    }
                });
            })
            .on_click(cx.listener(move |table, e: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                table
                    .delegate_mut()
                    .set_selected_paths(vec![select_path.clone()], Some(select_path.clone()));
                table.set_right_clicked_row(None, cx);
                cx.emit(TableEvent::SelectRow(row_ix));
                if e.click_count() == 2 {
                    cx.emit(TableEvent::DoubleClickedRow(row_ix));
                }
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |table, _: &MouseDownEvent, _window, cx| {
                    if !is_selected {
                        table.delegate_mut().set_selected_paths(
                            vec![context_path.clone()],
                            Some(context_path.clone()),
                        );
                        cx.notify();
                    }
                }),
            )
    }

    fn loading(&self, _: &App) -> bool {
        self.loading
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let material = miaominal_settings::current_theme().material;
        let roles = material.roles;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        let Some(row) = self.rows.get(row_ix) else {
            return div().into_any_element();
        };

        let orig_col_ix = self.visible_col_map.get(col_ix).copied().unwrap_or(col_ix);

        match orig_col_ix {
            0 => {
                let depth = row.depth;
                let is_directory = row.is_directory;
                let is_expanded = row.is_expanded;
                let is_loading = row.is_loading_children;
                let collapse_path = row.path.clone();
                let expand_path = row.path.clone();
                let icon = if row.is_directory {
                    if row.is_expanded || row.is_loading_children {
                        AppIcon::FolderOpen
                    } else {
                        AppIcon::Folder
                    }
                } else {
                    AppIcon::File
                };
                let name_icon_size = px(18.0);
                let name = row.name.clone();
                let is_renaming = self.inline_rename_path.as_deref() == Some(row.path.as_str());

                if is_renaming
                    && let Some(inline_rename_input) = self.app_view.upgrade().map(|view| {
                        view.read(cx)
                            .workspace_forms
                            .sftp_browser
                            .inline_rename_input
                            .clone()
                    })
                {
                    return h_flex()
                        .w_full()
                        .h_full()
                        .items_center()
                        .gap_1()
                        .overflow_hidden()
                        .pl(px(4.0 + 16.0 * depth as f32))
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(16.0))
                                .flex()
                                .items_center()
                                .justify_center(),
                        )
                        .child(
                            div()
                                .w(px(20.0))
                                .h(px(20.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(Icon::new(icon).size(name_icon_size)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .child(
                                    Input::new(&inline_rename_input)
                                        .appearance(false)
                                        .border_1()
                                        .border_color(rgb(roles.primary))
                                        .small()
                                        .w_full(),
                                ),
                        )
                        .into_any_element();
                }

                h_flex()
                    .w_full()
                    .h_full()
                    .items_center()
                    .gap_1()
                    .overflow_hidden()
                    .pl(px(4.0 + 16.0 * depth as f32))
                    .child(
                        div()
                            .w(px(16.0))
                            .h(px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(is_directory && is_loading, |this| {
                                this.child(
                                    Icon::new(IconName::LoaderCircle)
                                        .small()
                                        .text_color(rgb(text_muted)),
                                )
                            })
                            .when(is_directory && !is_loading && is_expanded, |this| {
                                this.child(
                                    div()
                                        .id(("sftp-collapse", row_ix))
                                        .cursor_pointer()
                                        .child(
                                            Icon::new(IconName::ChevronDown)
                                                .small()
                                                .text_color(rgb(text_muted)),
                                        )
                                        .on_click(cx.listener(
                                            move |table, _event, _window, cx| {
                                                cx.stop_propagation();
                                                table
                                                    .delegate_mut()
                                                    .collapse_directory(collapse_path.clone());
                                                cx.notify();
                                            },
                                        )),
                                )
                            })
                            .when(is_directory && !is_loading && !is_expanded, |this| {
                                this.child(
                                    div()
                                        .id(("sftp-expand", row_ix))
                                        .cursor_pointer()
                                        .child(
                                            Icon::new(IconName::ChevronRight)
                                                .small()
                                                .text_color(rgb(text_muted)),
                                        )
                                        .on_click(cx.listener(
                                            move |table, _event, _window, cx| {
                                                cx.stop_propagation();
                                                let delegate = table.delegate_mut();
                                                let app_view = delegate.app_view.clone();
                                                let side = delegate.side;
                                                let tab_id = delegate.tab_id;
                                                let path = expand_path.clone();
                                                if delegate.children_source.contains_key(&path) {
                                                    delegate.expand_cached_directory(path);
                                                } else {
                                                    delegate.loading_paths.insert(path.clone());
                                                    delegate.apply_sort();
                                                    // Defer via spawn so the current TableState
                                                    // update completes before expand_sftp_directory
                                                    // attempts to update this same entity again.
                                                    cx.spawn(async move |_this, cx| {
                                                        cx.update(|app| {
                                                            app_view
                                                                .update(app, |view, cx| {
                                                                    view.expand_sftp_directory(
                                                                        tab_id, side, path, cx,
                                                                    );
                                                                })
                                                                .ok();
                                                        });
                                                    })
                                                    .detach();
                                                }
                                                cx.notify();
                                            },
                                        )),
                                )
                            }),
                    )
                    .child(
                        div()
                            .w(px(20.0))
                            .h(px(20.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(icon).size(name_icon_size)),
                    )
                    .child(div().flex_1().min_w(px(0.0)).overflow_hidden().child(name))
                    .into_any_element()
            }
            1 => div()
                .h_full()
                .flex()
                .items_center()
                .justify_end()
                .child(format_byte_size(row.size))
                .into_any_element(),
            2 => div()
                .h_full()
                .flex()
                .items_center()
                .child(row.type_label.clone())
                .into_any_element(),
            3 => div()
                .h_full()
                .flex()
                .items_center()
                .child(format_local_timestamp(row.modified))
                .into_any_element(),
            4 => div()
                .h_full()
                .flex()
                .items_center()
                .child(row.attributes.clone().unwrap_or_else(|| "--".into()))
                .into_any_element(),
            _ => div()
                .h_full()
                .flex()
                .items_center()
                .child(row.owner.clone().unwrap_or_else(|| "--".into()))
                .into_any_element(),
        }
    }
}
