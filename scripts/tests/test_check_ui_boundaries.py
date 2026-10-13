"""Regression fixtures for architecture bypasses, independent of Cargo."""

from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_ui_boundaries import check_source, check_tree, lex, module_sources


class OwnerBoundaryTests(unittest.TestCase):
    def rules(self, source, module=("crate", "ui", "features", "hosts", "view")):
        path = "crates/miaominal-ui/src/ui/" + "/".join(module[2:]) + ".rs"
        return {failure.rule for failure in check_source(path, source, module)}

    def test_grouped_multiline_shell_import_is_rejected(self):
        self.assertIn("SHELL_DEPENDENCY", self.rules("""
            use crate::{ui::{
                shell::{AppView as WindowOwner, metrics::Layout},
                components::HintedInput,
            }};
        """))

    def test_relative_sibling_import_is_rejected(self):
        self.assertEqual(self.rules("use super::super::settings::SettingsController;"), {"SIBLING_FEATURE"})

    def test_alias_cannot_hide_a_qualified_owner_dependency(self):
        self.assertIn("SHELL_DEPENDENCY", self.rules("""
            use crate::ui as desktop;
            fn route() { desktop::shell::open_settings(); }
        """))

    def test_forward_and_chained_import_aliases_are_order_independent(self):
        self.assertIn("SHELL_DEPENDENCY", self.rules("""
            use desktop::shell as window;
            use crate::ui as desktop;
            fn route() { window::open_settings(); }
        """))

    def test_cyclic_aliases_terminate(self):
        self.assertEqual(self.rules("use one as two; use two as one;"), set())

    def test_literal_and_nested_comment_text_are_ignored(self):
        self.assertEqual(self.rules('''
            // use crate::ui::shell::AppView;
            /* outer /* nested AppView */ crate::ui::shell::State */
            const NOTE: &str = r###"use crate::ui::shell::AppView; \" /*"###;
            const NORMAL: &str = "AppView // use crate::ui::shell";
            let quote = '\"';
        '''), set())

    def test_tokens_preserve_lines_and_lifetimes(self):
        tokens = lex("// comment\nfn borrowed<'a>() -> &'a str { \"text\" }")
        self.assertEqual(tokens[0].line, 2)
        self.assertIn("'a", [token.value for token in tokens])

    def test_owner_relative_import_and_contract_ports_are_allowed(self):
        self.assertEqual(self.rules("""
            use super::HostsController;
            use crate::ui::contracts::{TabId, session::SessionTerminalPort};
            use crate::ui::components::{dialog::DialogConfig, HintedInput};
        """), set())

    def test_contract_cannot_alias_feature_type_or_use_feature_in_field(self):
        module = ("crate", "ui", "contracts", "sftp")
        self.assertIn("CONTRACT_DIRECTION", self.rules("""
            pub(in crate::ui) use crate::ui::features::sftp::TransferredSftpTab;
            pub(crate) type Transfer = crate::ui::features::sftp::TransferredSftpTab;
        """, module))

    def test_component_cannot_depend_on_business_owner(self):
        self.assertEqual(self.rules("use crate::ui::features::hosts::HostsController;", ("crate", "ui", "components", "picker")), {"COMPONENT_DIRECTION"})

    def test_feature_needs_application_service_port(self):
        self.assertEqual(self.rules("fn check() { crate::ui::application::application_state(cx); }"), {"FEATURE_DIRECTION"})

    def test_shell_can_compose_explicit_feature_facade(self):
        module = ("crate", "ui", "shell", "bootstrap")
        self.assertEqual(self.rules("use crate::ui::features::session::{SessionController, terminal_cell_width};", module), set())

    def test_shell_business_dialog_variants_cannot_return(self):
        self.assertEqual(self.rules("enum DialogOverlaySnapshot { HostDelete(Host) }", ("crate", "ui", "shell", "state")), {"BUSINESS_DIALOG_DISPATCH"})

    def test_shell_cannot_reach_private_feature_module(self):
        self.assertEqual(self.rules("use crate::ui::features::session::view::canvas::Canvas;", ("crate", "ui", "shell", "terminal")), {"FEATURE_INTERNAL"})

    def test_feature_wildcard_facade_is_rejected(self):
        self.assertIn("BROAD_FEATURE_EXPORT", self.rules("pub(in crate::ui) use controller::*;", ("crate", "ui", "features", "hosts")))

    def test_public_internal_module_is_rejected_but_owner_scope_is_allowed(self):
        module = ("crate", "ui", "features", "hosts")
        self.assertEqual(self.rules("pub(in crate::ui) mod forms;", module), {"PUBLIC_FEATURE_MODULE"})
        self.assertEqual(self.rules("pub(in crate::ui::features::hosts) mod forms;", module), set())

    def test_external_mutable_state_return_is_rejected(self):
        self.assertEqual(self.rules("""
            pub(in crate::ui) fn tab_mut<'a>(&'a self, id: TabId)
                -> Option<RefMut<'a, SessionTabState>> { todo!() }
            pub(crate) fn state_mut(&mut self) -> &mut State { todo!() }
        """), {"MUTABLE_ESCAPE"})

    def test_mutation_methods_and_owned_callbacks_are_allowed(self):
        self.assertEqual(self.rules("""
            pub(in crate::ui) fn confirm(&mut self, window: &mut Window, cx: &mut App) -> bool { true }
            pub(in crate::ui) fn callback() -> impl Fn(&mut Window, &mut App) { todo!() }
            pub(in crate::ui::features::hosts) fn state_mut(&mut self) -> RefMut<'_, State> { todo!() }
        """), set())

    def test_public_trait_cannot_hide_a_mutable_escape(self):
        self.assertEqual(self.rules("""
            pub(in crate::ui) trait RuntimeAccess {
                fn state_mut(&mut self) -> &mut RuntimeState;
            }
        """), {"MUTABLE_ESCAPE"})

    def test_only_session_input_adapter_gets_transient_mutable_borrow(self):
        module = ("crate", "ui", "features", "session", "input", "gestures")
        adapter = """
            pub(crate) trait TerminalInputHost {
                fn active_terminal_pane_mut(&mut self) -> &mut PaneViewState;
                fn terminal_drag_mut(&mut self) -> &mut TerminalDragState;
            }
        """
        self.assertEqual(self.rules(adapter, module), set())
        self.assertEqual(self.rules(adapter), {"MUTABLE_ESCAPE"})

    def test_public_mutable_container_field_and_alias_are_rejected(self):
        self.assertEqual(self.rules("""
            pub(crate) struct HostHandle { pub(crate) state: Rc<RefCell<State>>, }
            pub(crate) type StateHandle = Rc<RefCell<State>>;
            pub(crate) fn state(&self) -> Rc<RefCell<State>> { todo!() }
        """), {"MUTABLE_ESCAPE"})

    def test_owner_scoped_wildcard_is_allowed(self):
        self.assertEqual(self.rules("pub(in crate::ui::features::hosts) use super::*;"), set())

    def test_inline_module_relative_path_has_correct_depth(self):
        self.assertEqual(self.rules("mod tests { use super::super::HostsController; }"), set())
        self.assertIn("SIBLING_FEATURE", self.rules("mod tests { use super::super::super::settings::Controller; }"))

    def test_source_redirection_cannot_hide_shell_dependency(self):
        self.assertEqual(self.rules('#[path = "../../../shell/controller.rs"] mod controller;'), {"SOURCE_REDIRECT"})
        self.assertEqual(self.rules('#[path = "cards.rs"] mod cards;'), set())

    def test_tree_covers_flat_owner_roots_and_unreferenced_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ui = root / "crates/miaominal-ui/src/ui"
            (ui / "features/hosts").mkdir(parents=True)
            (ui / "application.rs").write_text("use crate::ui::shell::AppView;", encoding="utf-8")
            (ui / "features/hosts/dormant.rs").write_text("use crate::ui::features::settings::Controller;", encoding="utf-8")
            failures = check_tree(root)
            self.assertEqual({failure.rule for failure in failures}, {"SHELL_DEPENDENCY", "APP_VIEW_DEPENDENCY", "SIBLING_FEATURE"})

    def test_obsolete_shell_owner_is_rejected_even_when_empty_and_unused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            controllers = root / "crates/miaominal-ui/src/ui/shell/controllers"
            controllers.mkdir(parents=True)
            (controllers / "session.rs").write_text("", encoding="utf-8")
            self.assertEqual({failure.rule for failure in check_tree(root)}, {"OBSOLETE_OWNER_SOURCE"})

    def test_path_attribute_uses_declared_module_for_relative_resolution(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ui = root / "crates/miaominal-ui/src/ui"
            (ui / "features/hosts").mkdir(parents=True)
            ui.with_suffix(".rs").write_text("mod features;", encoding="utf-8")
            (ui / "features.rs").write_text("mod hosts;", encoding="utf-8")
            (ui / "features/hosts.rs").write_text("mod controller;", encoding="utf-8")
            (ui / "features/hosts/controller.rs").write_text('#[path = "dialogs.rs"] mod dialogs;', encoding="utf-8")
            dialogs = ui / "features/hosts/dialogs.rs"
            dialogs.write_text("use super::super::super::settings::Controller;", encoding="utf-8")
            modules = module_sources(ui)
            self.assertEqual(modules[dialogs.resolve()], ("crate", "ui", "features", "hosts", "controller", "dialogs"))
            self.assertIn("SIBLING_FEATURE", {failure.rule for failure in check_tree(root)})

    def test_import_of_internal_module_itself_is_rejected(self):
        module = ("crate", "ui", "shell", "terminal")
        failures = check_source("shell/terminal.rs", "use crate::ui::features::session::input as terminal;", module, {("crate", "ui", "features", "session", "input")})
        self.assertEqual({failure.rule for failure in failures}, {"FEATURE_INTERNAL"})


if __name__ == "__main__":
    unittest.main()
