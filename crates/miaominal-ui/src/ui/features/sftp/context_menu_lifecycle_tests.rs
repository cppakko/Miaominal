//! Controller-backed pointer coverage; the production remote panel supplies every menu/input.
use super::*;
use crate::ui::contracts::catalog::CatalogReader;
use crate::ui::contracts::session::SessionQuerySource;
use gpui_kit::component::Root;
use gpui_kit::test::{ClickOptions, TestEventExt, TestQueryExt, TestWindowExt};
use gpui_kit::{TestAppContext, VisualTestContext, point};
use miaominal_core::proxy::ProxyProfile;
use miaominal_secrets::SecretStore;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use std::rc::Rc;

const TAB: TabId = TabId::new(1);
const TARGET: &str = "/files/bravo.txt";

struct Sources(SessionProfile);

impl CatalogReader for Sources {
    fn profiles(&self) -> Vec<SessionProfile> {
        vec![self.0.clone()]
    }

    fn profile(&self, id: &str) -> Option<SessionProfile> {
        (id == self.0.id).then(|| self.0.clone())
    }

    fn proxies(&self) -> Vec<ProxyProfile> {
        Vec::new()
    }
}

impl SessionQuerySource for Sources {
    fn active_profile_id(&self) -> Option<String> {
        Some(self.0.id.clone())
    }

    fn resolved_profile_for_session(
        &self,
        tab_id: TabId,
        _: &CatalogQueryPort,
    ) -> Option<SessionProfile> {
        (tab_id == TAB).then(|| self.0.clone())
    }
}

struct RemotePanelHarness {
    controller: Entity<SftpController>,
    _subscription: Subscription,
}

impl Render for RemotePanelHarness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().flex_col().child(
            self.controller
                .read(cx)
                .render_remote_browser_panel(self.controller.clone(), TAB)
                .expect("fixture remote SFTP tab"),
        )
    }
}

struct Fixture<'a> {
    controller: Entity<SftpController>,
    cx: &'a mut VisualTestContext,
    // No connection is started, and no real credentials or known-hosts file is touched.
    _runtime: tokio::runtime::Runtime,
    _directory: tempfile::TempDir,
}

fn fixture(cx: &mut TestAppContext) -> Fixture<'_> {
    cx.update(gpui_kit::init);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let service = SftpService::new(
        runtime.handle().clone(),
        SecretStore::new_locked_vault(),
        KnownHostsStore::with_path(directory.path().join("known_hosts")),
    );
    let captured = Rc::new(RefCell::new(None));
    let output = captured.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let profile = SessionProfile::blank("sftp-menu-lifecycle", 1);
        let sources = Rc::new(Sources(profile.clone()));
        let controller = cx.new(|cx| {
            SftpController::new(
                SftpControllerArgs {
                    service,
                    local_hidden_columns: vec![4, 5],
                    remote_hidden_columns: vec![4, 5],
                },
                CatalogQueryPort::new(sources.clone()),
                SessionQueryPort::new(sources),
                window,
                cx,
            )
        });
        controller.update(cx, |owner, cx| {
            let mut tab = SftpTabState::new(&profile);
            tab.connected = true;
            tab.loading_remote = false;
            tab.remote_path = "/files".into();
            tab.remote_entries = ["alpha.txt", "bravo.txt"]
                .map(|filename| SftpEntry {
                    filename: filename.into(),
                    path: format!("/files/{filename}"),
                    kind: miaominal_sftp::SftpEntryKind::File,
                    size: Some(1),
                    modified: None,
                    attributes: None,
                    owner: None,
                })
                .to_vec();
            tab.selected_remote_path = Some("/files/alpha.txt".into());
            tab.selected_remote_paths = vec!["/files/alpha.txt".into()];
            owner.tabs.borrow_mut().insert(TAB, tab);
            owner.sync_tables_for_tab(TAB, cx);
            owner.sync_path_inputs_for_tab(TAB, window, cx);
        });
        output.replace(Some(controller.clone()));
        let view = cx.new(|cx| RemotePanelHarness {
            _subscription: cx.observe(&controller, |_, _, cx| cx.notify()),
            controller,
        });
        Root::new(view, window, cx)
    });
    let controller = captured.borrow().as_ref().unwrap().clone();
    cx.run_until_parked();
    Fixture {
        controller,
        cx,
        _runtime: runtime,
        _directory: directory,
    }
}

fn open_target_menu(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        let row_height = gpui_kit::component::Size::Small.table_row_height();
        window.click_with_options(
            "table",
            ClickOptions::new()
                .with_button(MouseButton::Right)
                // Header, then alpha, then the initially-unselected bravo row.
                .with_offset(point(px(100.), row_height * 2.5)),
            cx,
        );
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_some());
        assert!(
            window
                .try_find_by_label(&i18n::string("sftp.menu.download"))
                .is_some(),
            "the production file menu must contain Download"
        );
        assert!(
            window
                .try_find_by_label(&i18n::string("sftp.menu.rename"))
                .is_some(),
            "a newly-selected single file must offer Rename"
        );
    });
}

fn click_rename(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let target = window
            .find_by_label(&i18n::string("sftp.menu.rename"))
            .bounds()
            .center();
        window.pointer_move(target, None, cx);
        window.pointer_down(target, MouseButton::Left, cx);
        window.pointer_up(target, MouseButton::Left, cx);
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn sftp_controller_right_click_selects_unselected_file_and_runs_rename(cx: &mut TestAppContext) {
    let Fixture {
        controller,
        cx,
        _runtime,
        _directory,
    } = fixture(cx);
    open_target_menu(cx);
    controller.read_with(cx, |owner, _| {
        let tab = owner.tab(TAB).unwrap();
        assert_eq!(tab.selected_remote_paths, vec![TARGET.to_string()]);
        assert_eq!(tab.selected_remote_path.as_deref(), Some(TARGET));
    });

    click_rename(cx);
    controller.read_with(cx, |owner, cx| {
        assert!(
            matches!(owner.inline_rename(TAB), Some(InlineRenameState::Remote { from, .. }) if from == TARGET),
            "the menu command must rename the newly-right-clicked file, not the old selection"
        );
        assert_eq!(owner.inline_rename_input().read(cx).value(), "bravo.txt");
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        let input = controller.read(cx).inline_rename_input();
        assert!(input.focus_handle(cx).is_focused(window));
    });
}

#[gpui_kit::test]
fn sftp_controller_rename_input_right_click_does_not_cancel_on_blur(cx: &mut TestAppContext) {
    let Fixture {
        controller,
        cx,
        _runtime,
        _directory,
    } = fixture(cx);
    open_target_menu(cx);
    click_rename(cx);
    let input = controller.read_with(cx, |owner, _| owner.inline_rename_input());
    let position = cx.update(|window, cx| {
        window.render_frame(cx);
        input.read(cx).input_bounds().center()
    });

    // Separate down/up: the original table-empty-menu focus bug cancels rename
    // on MouseDown, before the input can request its edit menu on MouseUp.
    cx.update(|window, cx| {
        window.pointer_move(position, None, cx);
        window.pointer_down(position, MouseButton::Right, cx);
    });
    cx.run_until_parked();
    controller.read_with(cx, |owner, _| {
        assert!(
            owner.has_inline_rename(TAB),
            "right down must not cancel rename"
        );
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(input.focus_handle(cx).is_focused(window));
        assert!(
            window.try_find("popup-menu").is_none(),
            "neither the SFTP host nor the table's empty menu may replace the input"
        );
        window.pointer_up(position, MouseButton::Right, cx);
    });
    cx.run_until_parked();
    controller.read_with(cx, |owner, _| {
        assert!(
            owner.has_inline_rename(TAB),
            "right up must preserve rename"
        );
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(input.read(cx).has_selection_focus(window, cx));
        // On Linux, dismiss the drawn native-menu fallback; on Windows/macOS
        // the headless platform has no OS popup handle and this is a harmless key.
        window.press("escape", cx);
        window.input("-draft", cx);
    });
    cx.run_until_parked();
    controller.read_with(cx, |owner, cx| {
        assert!(owner.has_inline_rename(TAB));
        assert!(
            owner
                .inline_rename_input()
                .read(cx)
                .value()
                .contains("-draft")
        );
        assert_eq!(
            owner.tab(TAB).unwrap().selected_remote_paths,
            vec![TARGET.to_string()]
        );
    });
    // NativeMenu uses the OS backend on Windows/macOS; its visible item list and
    // selecting Copy/Paste still require a real-window platform smoke test.
}

#[gpui_kit::test]
fn sftp_controller_path_input_right_click_keeps_input_commands_reachable(cx: &mut TestAppContext) {
    let Fixture {
        controller,
        cx,
        _runtime,
        _directory,
    } = fixture(cx);
    // Initial editing state is fixture setup; the gesture is dispatched to the
    // real path Input in the production panel, outside the table menu host.
    controller.update(cx, |owner, cx| {
        owner.set_remote_path_editing(true);
        cx.notify();
    });
    cx.run_until_parked();
    let input = controller.read_with(cx, |owner, _| owner.remote_path_input());
    cx.update(|window, cx| {
        window.render_frame(cx);
        let target = input.read(cx).input_bounds().center();
        window.pointer_move(target, None, cx);
        window.pointer_down(target, MouseButton::Right, cx);
        window.pointer_up(target, MouseButton::Right, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(input.read(cx).has_selection_focus(window, cx));
        assert!(
            window
                .try_find_by_label(&i18n::string("sftp.menu.download"))
                .is_none(),
            "path input must not be replaced by the SFTP file menu"
        );
        window.press("escape", cx);
        window.input("-draft", cx);
    });
    cx.run_until_parked();
    controller.read_with(cx, |owner, cx| {
        assert!(owner.remote_path_editing());
        assert!(
            owner
                .remote_path_input()
                .read(cx)
                .value()
                .contains("-draft")
        );
        assert_eq!(owner.tab(TAB).unwrap().remote_path, "/files");
    });
}
