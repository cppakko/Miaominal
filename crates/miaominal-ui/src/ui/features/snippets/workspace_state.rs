use super::*;
use crate::ui::contracts::snippets::TransferredSnippetsUi;

impl SnippetsController {
    pub(in crate::ui::features::snippets) fn toggle_workspace_package(
        &self,
        package: String,
        cx: &mut Context<Self>,
    ) {
        let mut selected = self.workspace_package_filter.borrow_mut();
        *selected = if selected.as_deref() == Some(package.as_str()) {
            None
        } else {
            Some(package)
        };
        let message = selected.as_ref().map_or_else(
            || i18n::string("snippets.messages.viewing_all"),
            |package| {
                i18n::string_args(
                    "snippets.messages.filtering_by_package",
                    &[("package", package)],
                )
            },
        );
        drop(selected);
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui) fn take_workspace_ui_for_transfer(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TransferredSnippetsUi {
        let filter = self.workspace_filter_input.read(cx).value().to_string();
        let package = self.workspace_package_filter.borrow_mut().take();
        set_input_value(&self.workspace_filter_input, "", window, cx);
        TransferredSnippetsUi::new(filter, package)
    }

    pub(in crate::ui) fn restore_workspace_ui_after_transfer(
        &self,
        ui: TransferredSnippetsUi,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (filter, package) = ui.into_parts();
        *self.workspace_package_filter.borrow_mut() = package;
        set_input_value(&self.workspace_filter_input, filter, window, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::{SearchInputStyle, search_filter_input};
    use crate::ui::contracts::snippets::{SnippetCatalog, SnippetCommitOperation};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AnyWindowHandle, Bounds, Render, TestAppContext, WindowBounds, WindowOptions, div, px, size,
    };
    use std::rc::Rc;

    struct CatalogFixture {
        records: Vec<SnippetRecord>,
        writes: Cell<usize>,
    }

    impl CatalogFixture {
        fn new() -> Rc<Self> {
            Rc::new(Self {
                records: vec![SnippetRecord {
                    id: "deploy".into(),
                    description: "部署脚本".into(),
                    package: "Operations".into(),
                    language: "bash".into(),
                    script: "echo 中文".into(),
                }],
                writes: Cell::new(0),
            })
        }
    }

    impl SnippetCatalog for CatalogFixture {
        fn snapshot(&self, _: &App) -> Vec<SnippetRecord> {
            self.records.clone()
        }

        fn commit(&self, _: &mut App, _: SnippetCommitOperation) -> Result<Vec<SnippetRecord>> {
            self.writes.set(self.writes.get() + 1);
            anyhow::bail!("workspace filtering must not persist snippet records")
        }
    }

    struct Harness {
        owner: Entity<SnippetsController>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(search_filter_input(
                &self.owner.read(cx).workspace_filter_input,
                SearchInputStyle::Compact,
                None,
            ))
        }
    }

    fn owner_window(
        cx: &mut TestAppContext,
        catalog: Rc<CatalogFixture>,
    ) -> (AnyWindowHandle, Entity<SnippetsController>) {
        let (handle, harness) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(480.), px(300.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| Harness {
                        owner: cx.new(|cx| {
                            SnippetsController::new(
                                SnippetCatalogPort::new(catalog),
                                None,
                                None,
                                window,
                                cx,
                            )
                        }),
                    })
                },
            )
            .expect("open snippets workspace test window")
        });
        let owner = cx.update(|cx| harness.read(cx).owner.clone());
        (handle, owner)
    }

    fn filter_state(
        owner: &Entity<SnippetsController>,
        cx: &mut TestAppContext,
    ) -> (String, Option<String>) {
        cx.update(|cx| {
            let owner = owner.read(cx);
            (
                owner.workspace_filter_input.read(cx).value().to_string(),
                owner.workspace_package_filter.borrow().clone(),
            )
        })
    }

    #[gpui_kit::test]
    fn workspace_transfer_keeps_window_inputs_local_and_moves_only_filter_values(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let catalog = CatalogFixture::new();
        let (source_window, source) = owner_window(cx, catalog.clone());
        let (destination_window, destination) = owner_window(cx, catalog.clone());
        let source_input = cx.update(|cx| source.read(cx).workspace_filter_input.clone());
        let destination_input = cx.update(|cx| destination.read(cx).workspace_filter_input.clone());
        assert_ne!(source_input.entity_id(), destination_input.entity_id());

        cx.update_window(source_window, |_, window, cx| {
            window.render_frame(cx);
            window.click(("input", source_input.entity_id()), cx);
            window.input("echo 中文", cx);
            source.update(cx, |owner, cx| {
                owner.toggle_workspace_package("Operations".into(), cx)
            });
        })
        .unwrap();
        let transferred = cx
            .update_window(source_window, |_, window, cx| {
                source.update(cx, |owner, cx| {
                    owner.take_workspace_ui_for_transfer(window, cx)
                })
            })
            .unwrap();
        assert_eq!(filter_state(&source, cx), (String::new(), None));

        cx.update_window(source_window, |_, window, cx| {
            source.update(cx, |owner, cx| {
                set_input_value(&owner.workspace_filter_input, "source-only", window, cx);
                owner.toggle_workspace_package("Source".into(), cx);
            });
        })
        .unwrap();
        cx.update_window(destination_window, |_, window, cx| {
            destination.update(cx, |owner, cx| {
                owner.restore_workspace_ui_after_transfer(transferred, window, cx)
            });
            window.render_frame(cx);
            window.click(("input", destination_input.entity_id()), cx);
            window.input("x", cx);
            window.press("backspace", cx);
        })
        .unwrap();

        assert_eq!(
            filter_state(&destination, cx),
            ("echo 中文".into(), Some("Operations".into()))
        );
        assert_eq!(
            filter_state(&source, cx),
            ("source-only".into(), Some("Source".into()))
        );
        cx.update(|cx| {
            assert_eq!(
                source.read(cx).workspace_filter_input.entity_id(),
                source_input.entity_id()
            );
            assert_eq!(
                destination.read(cx).workspace_filter_input.entity_id(),
                destination_input.entity_id()
            );
            assert_eq!(source.read(cx).snippets(), catalog.records);
            assert_eq!(destination.read(cx).snippets(), catalog.records);
        });
        assert_eq!(catalog.writes.get(), 0);
    }

    #[gpui_kit::test]
    fn rejected_workspace_transfer_restores_filters_without_touching_catalog_page(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let catalog = CatalogFixture::new();
        let (window_handle, owner) = owner_window(cx, catalog.clone());
        cx.update_window(window_handle, |_, window, cx| {
            owner.update(cx, |owner, cx| {
                set_input_value(&owner.workspace_filter_input, "echo 中文", window, cx);
                set_input_value(
                    &owner.forms.borrow().filter_input,
                    "catalog-only",
                    window,
                    cx,
                );
                owner.catalog_view.borrow_mut().snippets_package_filter = Some("Catalog".into());
                owner.toggle_workspace_package("Operations".into(), cx);
                let captured = owner.take_workspace_ui_for_transfer(window, cx);
                assert!(owner.workspace_filter_input.read(cx).value().is_empty());
                assert!(owner.workspace_package_filter.borrow().is_none());
                owner.restore_workspace_ui_after_transfer(captured, window, cx);
                assert_eq!(owner.workspace_filter_input.read(cx).value(), "echo 中文");
                assert_eq!(
                    owner.workspace_package_filter.borrow().as_deref(),
                    Some("Operations")
                );
                assert_eq!(
                    owner.forms.borrow().filter_input.read(cx).value(),
                    "catalog-only"
                );
                assert_eq!(
                    owner
                        .catalog_view
                        .borrow()
                        .snippets_package_filter
                        .as_deref(),
                    Some("Catalog")
                );
                assert_eq!(owner.snippets(), catalog.records);
            });
        })
        .unwrap();
        assert_eq!(catalog.writes.get(), 0);
    }
}
