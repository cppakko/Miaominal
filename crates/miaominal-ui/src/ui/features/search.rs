//! Search owns matching, input, selection and focus; Shell supplies sanitized data.
use crate::ui::assets::AppIcon;
use crate::ui::components::{
    HintedInput,
    forms::{new_input_state, set_input_value},
    shell_tokens::{ShellColors, ShellMetrics, ShellTypography, space},
};
use crate::ui::contracts::search::SearchKind;
use crate::ui::contracts::search::{SearchEntry, SearchTarget};
use crate::ui::i18n;
use gpui_kit::component::{
    Disableable, FocusTrapElement as _, Icon, IconName, Selectable,
    button::{Button, ButtonVariants as _},
    input::{InputEvent, InputState},
    menu::{ContextMenuExt as _, PopupMenu},
    scroll::ScrollableElement as _,
};
use gpui_kit::{
    Context, Entity, EventEmitter, Focusable as _, InteractiveElement as _, IntoElement,
    MouseButton, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, TestSupportExt as _, WeakEntity, Window, div, prelude::*, px,
};
use std::rc::Rc;

pub(in crate::ui) enum SearchEvent {
    QueryChanged(String),
    Activate(SearchTarget),
}
/// Builds the context menu of one result; the owner of the target's actions supplies it.
pub(in crate::ui) type SearchContextMenuBuilder =
    Rc<dyn Fn(&SearchTarget, PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;
pub(in crate::ui) struct SearchView {
    input: Entity<InputState>,
    entries: Vec<SearchEntry>,
    results: Vec<SearchEntry>,
    query: String,
    open: bool,
    suppress_next_focus: bool,
    selected: usize,
    scroll: ScrollHandle,
    context_menu_builder: Option<SearchContextMenuBuilder>,
    context_menu: Option<WeakEntity<PopupMenu>>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<SearchEvent> for SearchView {}

fn matching_entries(entries: &[SearchEntry], query: &str, recent_limit: usize) -> Vec<SearchEntry> {
    let mut results: Vec<_> = entries
        .iter()
        .filter(|entry| {
            if query.trim().is_empty() {
                entry.recent().is_some()
            } else {
                entry.matches(query)
            }
        })
        .cloned()
        .collect();
    if query.trim().is_empty() {
        results.sort_by_key(|entry| std::cmp::Reverse(entry.recent()));
        results.truncate(recent_limit);
    } else {
        results.sort_by(|a, b| {
            a.kind()
                .cmp(&b.kind())
                .then_with(|| a.title().to_lowercase().cmp(&b.title().to_lowercase()))
                .then_with(|| a.id().cmp(b.id()))
        });
    }
    results
}
impl SearchView {
    pub(in crate::ui) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = new_input_state(i18n::string("search.placeholder"), "", false, window, cx);
        let subscription = cx.subscribe_in(&input, window, |this, input, event, _, cx| {
            match event {
                InputEvent::Change => {
                    let value = input.read(cx).value().to_string();
                    if value != this.query {
                        this.query = value.clone();
                        this.refresh();
                        this.selected = 0;
                        this.scroll.scroll_to_item(0);
                        this.open = true;
                        cx.emit(SearchEvent::QueryChanged(value));
                    }
                }
                InputEvent::Focus => {
                    if this.suppress_next_focus {
                        this.suppress_next_focus = false;
                    } else {
                        this.open = true;
                    }
                }
                _ => {}
            }
            cx.notify();
        });
        Self {
            input,
            entries: Vec::new(),
            results: Vec::new(),
            query: String::new(),
            open: false,
            suppress_next_focus: false,
            selected: 0,
            scroll: ScrollHandle::new(),
            context_menu_builder: None,
            context_menu: None,
            _subscriptions: vec![subscription],
        }
    }
    pub(in crate::ui) fn set_context_menu_builder(&mut self, builder: SearchContextMenuBuilder) {
        self.context_menu_builder = Some(builder);
    }
    /// Whether a result's context menu currently owns keyboard focus.
    fn context_menu_focused(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        self.context_menu
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .is_some_and(|menu| {
                let focus = menu.focus_handle(cx);
                focus.is_focused(window) || focus.contains_focused(window, cx)
            })
    }
    /// Closes the results and leaves the input unfocused. The blur is deferred
    /// so focus a closing menu hands back to the input is released as well,
    /// while focus that moved elsewhere (a dialog, another field) is kept.
    pub(in crate::ui) fn release_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        let input = self.input.clone();
        window.defer(cx, move |window, cx| {
            if input.focus_handle(cx).is_focused(window) {
                window.blur(cx);
            }
        });
        cx.notify();
    }
    fn refresh(&mut self) {
        let selected_id = self
            .results
            .get(self.selected)
            .map(|entry| entry.id().to_owned());
        self.results = matching_entries(
            &self.entries,
            &self.query,
            miaominal_settings::current_settings().recent_connections_count as usize,
        );
        self.selected = selected_id
            .and_then(|id| self.results.iter().position(|entry| entry.id() == id))
            .unwrap_or(0);
    }
    pub(in crate::ui) fn set_entries(&mut self, entries: Vec<SearchEntry>) {
        self.entries = entries;
        self.refresh();
    }
    pub(in crate::ui) fn load_query(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open = false;
        self.query = query.clone();
        set_input_value(&self.input, query, window, cx);
        self.refresh();
        self.selected = 0;
        self.scroll.scroll_to_item(0);
        cx.notify();
    }
    pub(in crate::ui) fn refresh_locale(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::components::forms::set_input_placeholder(
            &self.input,
            i18n::string("search.placeholder"),
            window,
            cx,
        );
    }
    pub(in crate::ui) fn is_open(&self) -> bool {
        self.open
    }
    pub(in crate::ui) fn dismiss(
        &mut self,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if restore_focus {
            self.suppress_next_focus = !self.input.focus_handle(cx).is_focused(window);
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
        self.open = false;
        cx.notify();
    }
    pub(in crate::ui) fn handle_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.open {
            return false;
        }
        // An open result menu handles its own navigation, confirm and cancel.
        if self.context_menu_focused(window, cx) {
            return false;
        }
        match key {
            "escape" => self.dismiss(true, window, cx),
            "down" | "up" => {
                if !self.results.is_empty() {
                    self.selected = if key == "down" {
                        (self.selected + 1) % self.results.len()
                    } else {
                        (self.selected + self.results.len() - 1) % self.results.len()
                    };
                    self.scroll.scroll_to_item(self.selected);
                }
                cx.notify();
            }
            "enter" => {
                // Buttons retain their native Enter behavior when reached with Tab.
                if !self.input.focus_handle(cx).is_focused(window) {
                    return false;
                }
                if let Some(entry) = self.results.get(self.selected) {
                    let target = entry.target().clone();
                    self.open = false;
                    cx.emit(SearchEvent::Activate(target));
                    cx.notify();
                }
            }
            _ => return false,
        }
        true
    }
}
impl Render for SearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = ShellColors::current();
        let focus = self.input.focus_handle(cx);
        let input = HintedInput::new(&self.input)
            .appearance(false)
            .text_metrics(
                miaominal_settings::scaled_font_size(ShellTypography::TITLE_SIZE),
                miaominal_settings::scaled_line_height(ShellTypography::TITLE_LINE_HEIGHT),
            )
            .hint_left(miaominal_settings::scaled_font_size(16.0))
            .hint_color(colors.muted)
            .h(space(ShellMetrics::SEARCH_HEIGHT))
            .w_full()
            .p_0()
            .px(space(16.0))
            .when(!self.query.is_empty(), |input| {
                input.suffix(
                    Button::new("search-clear")
                        .ghost()
                        .child(Icon::new(IconName::Close).size(space(20.0)))
                        .accessibility_label(i18n::string("search.clear"))
                        .size(space(32.0))
                        .disabled(self.query.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| {
                            set_input_value(&this.input, "", window, cx);
                            this.input.update(cx, |input, cx| input.focus(window, cx));
                        })),
                )
            });
        let mut content = div()
            .id("management-search")
            .occlude()
            // Pointer presses elsewhere release the field, as on any desktop surface.
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                if this.input.focus_handle(cx).is_focused(window) {
                    this.release_focus(window, cx);
                }
            }))
            .w_full()
            .flex()
            .flex_col()
            .gap(space(8.0))
            .child(
                div()
                    .id("search-input-region")
                    .test_support()
                    .capture_any_mouse_down(cx.listener(|this, _, _, cx| {
                        this.open = true;
                        cx.notify();
                    }))
                    .h(space(ShellMetrics::SEARCH_HEIGHT))
                    .relative()
                    .rounded_full()
                    .bg(if self.open { colors.hover } else { colors.card })
                    .px(space(20.0))
                    .flex()
                    .items_center()
                    .gap(space(12.0))
                    .child(
                        Icon::new(IconName::Search)
                            .size(space(24.0))
                            .text_color(colors.muted),
                    )
                    .child(div().flex_1().min_w_0().child(input))
                    .when(self.open, |region| {
                        region.child(
                            div()
                                .absolute()
                                .inset(space(-4.0))
                                .rounded_full()
                                .border_2()
                                .border_color(colors.primary),
                        )
                    }),
            );
        if self.open {
            let max_height = (f32::from(window.bounds().size.height)
                - f32::from(window.rem_size()) * 17.0)
                .max(100.0);
            let recent = self.query.trim().is_empty();
            let rows = self
                .results
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let heading =
                        index == 0 || (!recent && self.results[index - 1].kind() != entry.kind());
                    let target = entry.target().clone();
                    let menu_target = target.clone();
                    let menu_builder = self.context_menu_builder.clone();
                    let menu_owner = cx.weak_entity();
                    let selected = self.selected == index;
                    div()
                        .id(SharedString::from(format!("search-row:{}", entry.id())))
                        .when(heading, |row| {
                            row.child(
                                gpui_kit::component::h_flex()
                                    .px(space(12.0))
                                    .py(space(4.0))
                                    .gap(space(8.0))
                                    .text_size(space(ShellTypography::BODY_SIZE))
                                    .line_height(space(ShellTypography::SEARCH_GROUP_LINE_HEIGHT))
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .text_color(colors.primary)
                                    .child(i18n::string(if recent {
                                        "search.recent"
                                    } else {
                                        entry.kind().label_key()
                                    }))
                                    .child(div().font_weight(gpui_kit::FontWeight::NORMAL).text_color(colors.muted).child(
                                        self.results.iter().filter(|result| recent || result.kind() == entry.kind()).count().to_string(),
                                    )),
                            )
                        })
                        .child(
                            div()
                                .id(SharedString::from(format!("search-result-item:{}", entry.id())))
                                .w_full()
                                // The keyboard selection follows the pointer, so the highlight
                                // marks the result under the mouse or the one whose menu is open
                                // rather than staying on the first result.
                                .on_mouse_move(cx.listener(move |this, _, window, cx| {
                                    if this.selected != index && !this.context_menu_focused(window, cx) {
                                        this.selected = index;
                                        cx.notify();
                                    }
                                }))
                                .capture_any_mouse_down(cx.listener(
                                    move |this, event: &gpui_kit::MouseDownEvent, _, cx| {
                                        if event.button == MouseButton::Right && this.selected != index {
                                            this.selected = index;
                                            cx.notify();
                                        }
                                    },
                                ))
                                .context_menu(move |menu, window, cx| {
                                    let Some(builder) = menu_builder.as_ref() else {
                                        return menu;
                                    };
                                    let menu_handle = cx.weak_entity();
                                    menu_owner
                                        .update(cx, |search, _| search.context_menu = Some(menu_handle))
                                        .ok();
                                    builder(&menu_target, menu, window, cx)
                                })
                                .child(
                            Button::new(SharedString::from(format!(
                                "search-result:{}",
                                entry.id()
                            )))
                            .ghost()
                            // Right-click opens the result menu; keep focus where it is so
                            // the menu hands it back to the search field on dismiss. Capture
                            // phase, so the press also never paints the pressed fill.
                            .capture_any_mouse_down(|event, window, _| {
                                if event.button == MouseButton::Right {
                                    window.prevent_default()
                                }
                            })
                            .accessibility_label(format!(
                                "{} · {}",
                                entry.title(),
                                entry.category()
                            ))
                            .selected(selected)
                            .w_full()
                            .h_auto()
                            .min_h(space(56.0))
                            .px(space(12.0))
                            .py(space(8.0))
                            .rounded(miaominal_settings::scaled_font_size(16.0))
                            .justify_start()
                            .when(selected, |button| {
                                button.bg(colors.card.blend(colors.foreground.opacity(0.12))).text_color(colors.foreground)
                            })
                            .child(
                                gpui_kit::component::h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap(space(8.0))
                                    .child(
                                        div().size(space(32.0)).flex_shrink_0().flex().items_center().justify_center()
                                        .text_color(colors.primary).child(Icon::new(match entry.kind() {
                                            SearchKind::Host => AppIcon::Computer,
                                            SearchKind::Key => AppIcon::Key,
                                            SearchKind::Forward => AppIcon::Forward,
                                            SearchKind::Snippet => AppIcon::Notebook,
                                            SearchKind::Trust => AppIcon::FingerPrint,
                                            SearchKind::Setting => AppIcon::Settings,
                                        })
                                        .size(space(20.0))),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .items_start()
                                            .child(
                                                div()
                                                    .w_full()
                                                    .truncate()
                                                    .text_size(space(ShellTypography::TITLE_SIZE))
                                                    .line_height(space(ShellTypography::TITLE_LINE_HEIGHT))
                                                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                                                    .child(entry.title().to_owned()),
                                            )
                                            .child(
                                                div()
                                                    .w_full()
                                                    .truncate()
                                                    .text_size(space(ShellTypography::SUPPORTING_SIZE))
                                                    .line_height(space(ShellTypography::SEARCH_CATEGORY_LINE_HEIGHT))
                                                    .text_color(colors.muted)
                                                    .child(entry.category().to_owned()),
                                            ),
                                    )
                                    .child(Icon::new(AppIcon::Next).size(space(20.0)).text_color(colors.muted)),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.open = false;
                                    cx.emit(SearchEvent::Activate(target.clone()));
                                    cx.notify();
                                },
                            )),
                                ),
                        )
                })
                .collect::<Vec<_>>();
            content = content.child(
                div()
                    .id("search-results")
                    .relative()
                    .max_h(px(max_height))
                    .track_scroll(&self.scroll)
                    .overflow_y_scroll()
                    .vertical_scrollbar(&self.scroll)
                    .p(space(4.0))
                    .rounded(space(28.0))
                    .bg(colors.card)
                    .shadow_lg()
                    .child(
                        gpui_kit::component::h_flex()
                            .min_h(space(32.0))
                            .px(space(12.0))
                            .py(space(8.0))
                            .justify_between()
                            .gap(space(12.0))
                            .text_size(space(12.0))
                            .line_height(space(16.0))
                            .text_color(colors.muted)
                            .child(i18n::string_args(
                                "search.result_count",
                                &[("count", &self.results.len().to_string())],
                            ))
                            .when(
                                f32::from(window.viewport_size().width)
                                    > f32::from(window.rem_size()) / 14.0 * 700.0,
                                |row| row.child(i18n::string("search.shortcuts")),
                            ),
                    )
                    .when(rows.is_empty(), |list| {
                        list.child(
                            div()
                                .p(space(24.0))
                                .text_color(colors.muted)
                                .child(i18n::string("search.empty")),
                        )
                    })
                    .children(rows),
            );
            content
                .focus_trap("management-search-focus", &focus)
                .into_any_element()
        } else {
            content.into_any_element()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contracts::search::SearchKind;
    fn host(id: &str, title: &str, recent: Option<u64>) -> SearchEntry {
        SearchEntry::new(
            id.into(),
            SearchKind::Host,
            title.into(),
            "Production".into(),
            "ssh Linux".into(),
            SearchTarget::Host(id.into()),
        )
        .with_recent(recent)
    }
    #[test]
    fn search_uses_all_words_case_insensitively_and_does_not_mutate_catalog() {
        let entries = vec![host("a", "Alpha", Some(2)), host("b", "Beta", Some(4))];
        assert_eq!(
            matching_entries(&entries, "ALPHA linux production", 4)[0].id(),
            "a"
        );
        assert!(matching_entries(&entries, "Alpha Beta", 4).is_empty());
        assert_eq!(entries.len(), 2);
    }
    #[test]
    fn empty_query_uses_real_recent_order_and_setting_limit() {
        let entries = vec![
            host("a", "Alpha", Some(2)),
            host("b", "Beta", Some(4)),
            host("c", "New", None),
        ];
        assert_eq!(matching_entries(&entries, " ", 1)[0].id(), "b");
        assert!(matching_entries(&entries, "", 0).is_empty());
    }
    #[test]
    fn replacing_snapshot_removes_deleted_targets() {
        let mut entries = vec![host("a", "Alpha", Some(2))];
        assert_eq!(matching_entries(&entries, "Alpha", 4).len(), 1);
        entries.clear();
        assert!(matching_entries(&entries, "Alpha", 4).is_empty());
    }

    #[test]
    fn aggregate_results_include_every_kind_and_only_limit_recents() {
        let entries = vec![
            host("host:a", "Shared", None),
            SearchEntry::new(
                "key:a".into(),
                SearchKind::Key,
                "Shared".into(),
                String::new(),
                String::new(),
                SearchTarget::Key("a".into()),
            ),
            SearchEntry::new(
                "forward:a".into(),
                SearchKind::Forward,
                "Shared".into(),
                String::new(),
                String::new(),
                SearchTarget::Forward {
                    profile: "a".into(),
                    rule: "b".into(),
                },
            ),
            SearchEntry::new(
                "snippet:a".into(),
                SearchKind::Snippet,
                "Shared".into(),
                String::new(),
                String::new(),
                SearchTarget::Snippet("a".into()),
            ),
            SearchEntry::new(
                "trust:a".into(),
                SearchKind::Trust,
                "Shared".into(),
                String::new(),
                String::new(),
                SearchTarget::Trust {
                    host: "example.test".into(),
                    port: 22,
                    fingerprint: "fixture".into(),
                },
            ),
            SearchEntry::new(
                "setting:a".into(),
                SearchKind::Setting,
                "Shared".into(),
                String::new(),
                String::new(),
                SearchTarget::Setting("settings.appearance.font_family.label"),
            ),
        ];
        let results = matching_entries(&entries, "SHARED", 1);
        assert_eq!(
            results.iter().map(SearchEntry::kind).collect::<Vec<_>>(),
            vec![
                SearchKind::Host,
                SearchKind::Key,
                SearchKind::Forward,
                SearchKind::Snippet,
                SearchKind::Trust,
                SearchKind::Setting
            ]
        );
        assert_eq!(
            matching_entries(&entries, "端口转发 profile", 1)[0].id(),
            "forward:a"
        );
        assert_eq!(
            matching_entries(&entries, "SSH profile", 1)[0].id(),
            "host:a"
        );
        assert_eq!(
            matching_entries(&entries, "settings shared", 1)[0].id(),
            "setting:a"
        );
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crate::ui::contracts::search::SearchKind;
    use gpui_kit::{TestAppContext, component::Root, test::TestWindowExt as _};

    struct NotificationSearchHost {
        search: Entity<SearchView>,
    }

    impl Render for NotificationSearchHost {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .relative()
                .child(div().w(px(420.0)).child(self.search.clone()))
                .child(div().absolute().bottom_4().right_4().child(
                    crate::ui::components::notifications::render_status_center_popover(
                        None, window, cx,
                    ),
                ))
                .child(
                    Button::new("outside-notifications")
                        .absolute()
                        .bottom_4()
                        .left_4()
                        .label("Outside"),
                )
        }
    }

    #[gpui_kit::test]
    fn closing_notifications_does_not_reopen_dismissed_search(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let captured = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = captured.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let search = cx.new(|cx| SearchView::new(window, cx));
            output.replace(Some(search.clone()));
            let host = cx.new(|_| NotificationSearchHost { search });
            Root::new(host, window, cx)
        });
        let search = captured.borrow().as_ref().unwrap().clone();
        cx.update(|window, cx| {
            window.render_frame(cx);
            let input_id = ("input", search.read(cx).input.entity_id());
            window.click(input_id, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(search.read(cx).is_open());
            search.update(cx, |search, cx| search.dismiss(true, window, cx));
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(!search.read(cx).is_open());
            assert!(search.read(cx).input.focus_handle(cx).is_focused(window));
            window.click("notification-center-trigger-button", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("notification-center-trigger-button", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(!search.read(cx).is_open());
        });
    }

    #[gpui_kit::test]
    fn right_click_opens_the_owner_menu_and_it_keeps_its_keys(cx: &mut TestAppContext) {
        use gpui_kit::component::menu::PopupMenuItem;
        use std::{cell::Cell, rc::Rc};
        cx.update(gpui_kit::init);
        let captured = Rc::new(std::cell::RefCell::new(None));
        let output = captured.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let search = cx.new(|cx| SearchView::new(window, cx));
            output.replace(Some(search.clone()));
            Root::new(search, window, cx)
        });
        let search = captured.borrow().as_ref().unwrap().clone();
        let acted = Rc::new(Cell::new(None::<SearchTarget>));
        let record = acted.clone();
        cx.update(|window, cx| {
            search.update(cx, |search, cx| {
                search.set_context_menu_builder(Rc::new(move |target, menu, _, _| {
                    let record = record.clone();
                    let target = target.clone();
                    menu.item(
                        PopupMenuItem::new("Act")
                            .on_click(move |_, _, _| record.set(Some(target.clone()))),
                    )
                }));
                search.set_entries(vec![
                    SearchEntry::new(
                        "host:a".into(),
                        SearchKind::Host,
                        "Alpha".into(),
                        "Work".into(),
                        String::new(),
                        SearchTarget::Host("a".into()),
                    ),
                    SearchEntry::new(
                        "host:b".into(),
                        SearchKind::Host,
                        "Alpha Two".into(),
                        "Work".into(),
                        String::new(),
                        SearchTarget::Host("b".into()),
                    ),
                ]);
                search.load_query("alpha".into(), window, cx);
            });
            window.render_frame(cx);
            window.click("search-input-region", cx);
        });
        cx.run_until_parked();
        let center = cx.update(|window, cx| {
            window.render_frame(cx);
            assert_eq!(search.read(cx).selected, 0);
            window.find("search-result:host:b").bounds().center()
        });
        cx.simulate_mouse_down(center, MouseButton::Right, Default::default());
        cx.simulate_mouse_up(center, MouseButton::Right, Default::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(search.read(cx).is_open());
            assert_eq!(
                search.read(cx).selected,
                1,
                "the highlight follows the right-clicked result"
            );
            assert!(search.read(cx).context_menu_focused(window, cx));
        });
        cx.simulate_keystrokes("down enter");
        cx.run_until_parked();
        assert_eq!(acted.take(), Some(SearchTarget::Host("b".into())));
    }

    #[gpui_kit::test]
    fn pressing_outside_releases_the_search_field(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let captured = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = captured.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let search = cx.new(|cx| SearchView::new(window, cx));
            output.replace(Some(search.clone()));
            let host = cx.new(|_| NotificationSearchHost { search });
            Root::new(host, window, cx)
        });
        let search = captured.borrow().as_ref().unwrap().clone();
        cx.update(|window, cx| {
            window.render_frame(cx);
            let input_id = ("input", search.read(cx).input.entity_id());
            window.click(input_id, cx);
        });
        cx.run_until_parked();
        let blank = cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(search.read(cx).is_open());
            assert!(search.read(cx).input.focus_handle(cx).is_focused(window));
            let region = window.find("search-input-region").bounds();
            gpui_kit::point(region.center().x, region.bottom() + px(400.0))
        });
        cx.simulate_click(blank, Default::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(!search.read(cx).is_open());
            assert!(!search.read(cx).input.focus_handle(cx).is_focused(window));
        });
    }

    #[gpui_kit::test]
    fn keyboard_selection_emits_one_target_without_running_a_connection(cx: &mut TestAppContext) {
        use std::{cell::RefCell, rc::Rc};
        cx.update(gpui_kit::init);
        let captured = Rc::new(RefCell::new(None));
        let output = captured.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let search = cx.new(|cx| SearchView::new(window, cx));
            output.replace(Some(search.clone()));
            Root::new(search, window, cx)
        });
        let search = captured.borrow().as_ref().unwrap().clone();
        let activated = Rc::new(RefCell::new(Vec::new()));
        let targets = activated.clone();
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&search, move |_, event, _| {
                if let SearchEvent::Activate(target) = event {
                    targets.borrow_mut().push(target.clone());
                }
            })
        });
        cx.update(|window, cx| {
            search.update(cx, |search, cx| {
                search.set_entries(vec![
                    SearchEntry::new(
                        "host:a".into(),
                        SearchKind::Host,
                        "Alpha".into(),
                        "Work".into(),
                        String::new(),
                        SearchTarget::Host("a".into()),
                    ),
                    SearchEntry::new(
                        "key:b".into(),
                        SearchKind::Key,
                        "Beta".into(),
                        "Work".into(),
                        String::new(),
                        SearchTarget::Key("b".into()),
                    ),
                ]);
                search.load_query("work".into(), window, cx);
            });
            window.render_frame(cx);
            window.click("search-input-region", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(search.read(cx).is_open());
            assert!(window.find("search-result:host:a").visible());
            assert!(window.find("search-result:key:b").visible());
            search.update(cx, |search, cx| {
                assert!(search.handle_key("down", window, cx));
                assert!(search.handle_key("enter", window, cx));
            });
        });
        cx.run_until_parked();
        assert_eq!(
            activated.borrow().as_slice(),
            &[SearchTarget::Key("b".into())]
        );
        cx.update(|_, cx| assert!(!search.read(cx).is_open()));
    }
    #[gpui_kit::test]
    fn dismiss_restores_input_focus_and_loading_another_query_does_not_reopen(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let captured = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = captured.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let search = cx.new(|cx| SearchView::new(window, cx));
            output.replace(Some(search.clone()));
            Root::new(search, window, cx)
        });
        let search = captured.borrow().as_ref().unwrap().clone();
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            let input_id = ("input", search.read(cx).input.entity_id());
            window.click(input_id, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(search.read(cx).is_open());
            search.update(cx, |search, cx| {
                assert!(search.handle_key("escape", window, cx));
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(!search.read(cx).is_open());
            assert!(search.read(cx).input.focus_handle(cx).is_focused(window));
            search.update(cx, |search, cx| {
                search.load_query("another tab".into(), window, cx)
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| assert!(!search.read(cx).is_open()));
    }
    #[gpui_kit::test]
    fn replacement_keeps_stable_selection_and_drops_deleted_result(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let search = cx.new(|cx| SearchView::new(window, cx));
            Root::new(search, window, cx)
        });
        cx.update(|window, cx| {
            let view = cx.new(|cx| SearchView::new(window, cx));
            view.update(cx, |search, _| {
                let entry = SearchEntry::new(
                    "host:a".into(),
                    SearchKind::Host,
                    "Alpha".into(),
                    "Work".into(),
                    String::new(),
                    SearchTarget::Host("a".into()),
                )
                .with_recent(Some(3));
                search.set_entries(vec![entry.clone()]);
                assert_eq!(search.results[0].id(), "host:a");
                search.set_entries(vec![entry]);
                assert_eq!(search.results[search.selected].id(), "host:a");
                search.set_entries(vec![]);
                assert!(search.results.is_empty());
            });
        });
    }
}
