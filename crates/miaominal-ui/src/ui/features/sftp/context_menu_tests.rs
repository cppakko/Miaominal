use super::*;
use gpui_kit::component::Root;
use gpui_kit::test::{ClickOptions, TestWindowExt};
use gpui_kit::{TestAppContext, VisualTestContext, point};
use std::cell::Cell;

struct MenuHarness {
    table: Entity<TableState<SftpBrowserTableDelegate>>,
    floating: bool,
    built_for: Rc<RefCell<Vec<Option<usize>>>>,
    acted: Rc<Cell<usize>>,
}

impl Render for MenuHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let table = self.table.clone();
        let built_for = self.built_for.clone();
        let acted = self.acted.clone();
        sftp_browser_section(SftpBrowserSection {
            section_id: "sftp-menu-fixture".into(),
            join: SftpPanelJoin::None,
            title: "Files".into(),
            show_title: true,
            icon: AppIcon::Folder,
            item_count: 3,
            selected_count: 0,
            path_bar: div(),
            toolbar: div(),
            content: div().flex_1().min_w_0().min_h_0().child(SftpBrowserTable {
                table: self.table.clone(),
                has_floating_menu: self.floating,
            }),
            table: self.table.clone(),
            menu_builder: move |menu: PopupMenu, _: &mut Window, cx: &mut Context<PopupMenu>| {
                if table.update(cx, |table, _| {
                    table.delegate_mut().take_nested_context_menu_requested()
                }) {
                    return menu;
                }
                let table_state = table.read(cx);
                built_for.borrow_mut().push(
                    table_state
                        .right_clicked_row()
                        .filter(|row_ix| table_state.delegate().row(*row_ix).is_some()),
                );
                let acted = acted.clone();
                menu.item(
                    PopupMenuItem::new("Run file command").on_click(move |_, _, _| {
                        acted.set(acted.get() + 1);
                    }),
                )
            },
        })
    }
}

type MenuFixture<'a> = (
    Entity<TableState<SftpBrowserTableDelegate>>,
    Rc<RefCell<Vec<Option<usize>>>>,
    Rc<Cell<usize>>,
    &'a mut VisualTestContext,
);

fn fixture(
    cx: &mut TestAppContext,
    side: SftpBrowserSide,
    floating: bool,
    empty: bool,
) -> MenuFixture<'_> {
    cx.update(gpui_kit::init);
    let captured = Rc::new(RefCell::new(None));
    let output = captured.clone();
    let built_for = Rc::new(RefCell::new(Vec::new()));
    let built_output = built_for.clone();
    let acted = Rc::new(Cell::new(0));
    let acted_output = acted.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let table = cx.new(|cx| {
            let mut delegate = SftpBrowserTableDelegate::new(side);
            if !empty {
                delegate.set_rows(
                    ["alpha", "bravo", "charlie"]
                        .map(|name| SftpBrowserTableRow {
                            path: format!("/files/{name}"),
                            name: name.into(),
                            is_directory: true,
                            kind: miaominal_sftp::SftpEntryKind::Directory,
                            size: None,
                            type_label: "Directory".into(),
                            modified: None,
                            attributes: None,
                            owner: None,
                            depth: 0,
                            is_expanded: false,
                            is_loading_children: false,
                        })
                        .to_vec(),
                    false,
                    TabId::new(1),
                );
                delegate.set_selected_paths(
                    vec!["/files/alpha".into(), "/files/bravo".into()],
                    Some("/files/alpha".into()),
                );
            }
            let mut table = TableState::new(delegate, window, cx).row_selectable(false);
            table.col_fixed = false;
            table
        });
        output.replace(Some(table.clone()));
        let view = cx.new(|_| MenuHarness {
            table,
            floating,
            built_for: built_output,
            acted: acted_output,
        });
        Root::new(view, window, cx)
    });
    (
        captured.borrow().as_ref().unwrap().clone(),
        built_for,
        acted,
        cx,
    )
}

fn right_click_table(cx: &mut VisualTestContext, row: Option<usize>) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        let bounds = window.find("table").bounds();
        assert!(
            bounds.size.height > px(240.),
            "the table must fill its flex viewport"
        );
        let height = gpui_kit::component::Size::Small.table_row_height();
        let y = row.map_or(bounds.size.height - height, |row| {
            height * (row as f32 + 1.5)
        });
        window.click_with_options(
            "table",
            ClickOptions::new()
                .with_button(MouseButton::Right)
                .with_offset(point(px(100.), y)),
            cx,
        );
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("popup-menu").is_some(),
            "SFTP right-click must show a menu"
        );
    });
}

#[gpui_kit::test]
fn sftp_context_menu_opens_on_selected_and_unselected_rows(cx: &mut TestAppContext) {
    let (table, built_for, acted, cx) = fixture(cx, SftpBrowserSide::Local, false, false);
    right_click_table(cx, Some(1));
    assert_eq!(*built_for.borrow(), vec![Some(1)]);
    cx.update(|_, cx| assert!(!table.read(cx).delegate().has_single_selection()));
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert_eq!(acted.get(), 1, "the menu must retain keyboard focus");
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        assert!(table.focus_handle(cx).is_focused(window));
    });
    right_click_table(cx, Some(2));
    assert_eq!(*built_for.borrow(), vec![Some(1), Some(2)]);
    cx.update(|_, cx| assert!(table.read(cx).delegate().has_single_selection()));
}

#[gpui_kit::test]
fn sftp_context_menu_opens_on_blank_space_after_a_row_menu(cx: &mut TestAppContext) {
    let (_, built_for, _, cx) = fixture(cx, SftpBrowserSide::Remote, true, false);
    right_click_table(cx, Some(0));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    right_click_table(cx, None);
    assert_eq!(*built_for.borrow(), vec![Some(0), None]);
}

#[gpui_kit::test]
fn sftp_context_menu_opens_in_an_empty_directory(cx: &mut TestAppContext) {
    let (_, built_for, _, cx) = fixture(cx, SftpBrowserSide::Remote, false, true);
    right_click_table(cx, None);
    assert_eq!(*built_for.borrow(), vec![None]);
}

#[gpui_kit::test]
fn sftp_context_menu_keeps_header_menu_separate(cx: &mut TestAppContext) {
    let (_, built_for, _, cx) = fixture(cx, SftpBrowserSide::Remote, false, false);
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click_with_options(
            "table",
            ClickOptions::new()
                .with_button(MouseButton::Right)
                .with_offset(point(px(100.), px(10.))),
            cx,
        );
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_some());
    });
    assert!(
        built_for.borrow().is_empty(),
        "header opens only its column menu"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    right_click_table(cx, Some(1));
    assert_eq!(*built_for.borrow(), vec![Some(1)]);
}

#[gpui_kit::test]
fn sftp_context_menu_clearance_is_directory_scoped_and_accepts_pointer_commands(
    cx: &mut TestAppContext,
) {
    let (table, built_for, acted, cx) = fixture(cx, SftpBrowserSide::Remote, true, false);
    right_click_table(cx, Some(3));
    assert_eq!(*built_for.borrow(), vec![None]);
    cx.update(|window, cx| {
        assert!(!table.read(cx).delegate().has_single_selection());
        window.within("popup-menu").click(0usize, cx);
    });
    cx.run_until_parked();
    assert_eq!(acted.get(), 1);
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
    });
}

#[gpui_kit::test]
fn sftp_context_menu_does_not_reenter_from_its_own_popup(cx: &mut TestAppContext) {
    let (_, built_for, _, cx) = fixture(cx, SftpBrowserSide::Remote, false, false);
    right_click_table(cx, Some(1));
    cx.update(|window, cx| window.right_click("popup-menu", cx));
    cx.run_until_parked();
    assert_eq!(*built_for.borrow(), vec![Some(1)]);
}

#[gpui_kit::test]
fn sftp_context_menu_opens_only_once_while_loading(cx: &mut TestAppContext) {
    let (table, built_for, _, cx) = fixture(cx, SftpBrowserSide::Remote, false, false);
    cx.update(|_, cx| {
        table.update(cx, |table, cx| {
            table.delegate_mut().set_loading(true);
            cx.notify();
        })
    });
    right_click_table(cx, None);
    assert_eq!(*built_for.borrow(), vec![None]);
}
