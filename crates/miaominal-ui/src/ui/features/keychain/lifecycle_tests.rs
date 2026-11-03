use super::*;
use crate::ui::contracts::catalog::CatalogReader;
use crate::ui::contracts::keychain::{ManagedKeyCommitOperation, ManagedKeyCommitter};
use crate::ui::contracts::overlay::OverlaySnapshot;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    Bounds, Render, TestAppContext, TestSupportExt as _, WindowBounds, WindowOptions, size,
};
use miaominal_core::keychain::ManagedKeySource;
use miaominal_core::proxy::ProxyProfile;
use miaominal_secrets::CredentialStore;
use miaominal_secrets::credential_backend::CredentialBackend;
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct EmptyCredentials;
impl CredentialBackend for EmptyCredentials {
    fn name(&self) -> &'static str {
        "keychain-lifecycle-tests"
    }
    fn get(&self, _: &str, _: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    fn set(&self, _: &str, _: &str, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn delete(&self, _: &str, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct CatalogFixture {
    keys: RefCell<Vec<ManagedKeyRecord>>,
    store: RefCell<Option<ManagedKeyStore>>,
    fail: Cell<bool>,
    commits: Cell<usize>,
}
impl CatalogReader for CatalogFixture {
    fn profiles(&self) -> Vec<SessionProfile> {
        Vec::new()
    }
    fn profile(&self, _: &str) -> Option<SessionProfile> {
        None
    }
    fn proxies(&self) -> Vec<ProxyProfile> {
        Vec::new()
    }
}
impl ManagedKeyCommitter for Rc<CatalogFixture> {
    fn commit(
        &self,
        _: &mut App,
        operation: ManagedKeyCommitOperation,
    ) -> Result<Vec<ManagedKeyRecord>> {
        if self.fail.get() {
            anyhow::bail!("catalog persistence unavailable");
        }
        let current = self.store.borrow().as_ref().unwrap().load()?;
        let keys = operation(current)?;
        if keys != *self.keys.borrow() {
            self.keys.replace(keys.clone());
            self.commits.set(self.commits.get() + 1);
        }
        Ok(keys)
    }
}

struct Harness {
    controller: Entity<KeychainController>,
    snapshot: Option<OverlaySnapshot>,
}
impl Render for Harness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().children(
            self.snapshot
                .as_ref()
                .map(|snapshot| snapshot.render(None, 700.0, cx)),
        )
    }
}

fn key(id: &str) -> ManagedKeyRecord {
    ManagedKeyRecord {
        id: id.into(),
        name: id.into(),
        algorithm: "Ed25519".into(),
        public_key: format!("ssh-ed25519 public-{id}"),
        source: ManagedKeySource::Imported,
    }
}

struct EmptyPageGeometryHarness {
    controller: Entity<KeychainController>,
}

impl Render for EmptyPageGeometryHarness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("keychain-page-viewport")
            .test_support()
            .size_full()
            .overflow_hidden()
            .child(self.controller.read(cx).render_keychain_page(
                self.controller.clone(),
                window.viewport_size().width.as_f32(),
                cx,
            ))
    }
}

fn assert_empty_keychain_page_geometry(cx: &mut TestAppContext, page_view: KeychainPageView) {
    cx.update(gpui_kit::init);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let catalog = Rc::new(CatalogFixture::default());
    let source = catalog.clone();
    let runtime_handle = runtime.handle().clone();
    let known_hosts = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    let (handle, _view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(900.0), px(720.0)),
                ))),
                ..Default::default()
            },
            cx,
            move |window, cx| {
                cx.new(|cx| {
                    let query_source = source.clone();
                    let controller = cx.new(|cx| {
                        let mut controller = KeychainController::new(
                            KeychainControllerArgs {
                                managed_keys: Vec::new(),
                                runtime: runtime_handle,
                                keychain_store: None,
                                secrets: SecretStore::with_credentials(
                                    CredentialStore::with_backend(
                                        "keychain-empty-geometry-tests",
                                        EmptyCredentials,
                                    ),
                                ),
                                known_hosts,
                                local_vault_status: LocalVaultStatus::Disabled,
                                managed_key_query: ManagedKeyQueryPort::new(move |_| {
                                    query_source.keys.borrow().clone()
                                }),
                                managed_key_commit: ManagedKeyCommitPort::new(source.clone()),
                            },
                            CatalogQueryPort::new(source.clone()),
                            window,
                            cx,
                        );
                        controller.page_view = page_view;
                        controller
                    });
                    EmptyPageGeometryHarness { controller }
                })
            },
        )
        .expect("open keychain empty-page geometry window")
    });

    for scale in [1.0, 2.0] {
        cx.simulate_window_scale_factor_change(handle, scale);
        // Restore the tall viewport too: the empty state's geometry must not
        // retain a minimum or a stale center from the shorter frame.
        for height in [720.0, 360.0, 720.0] {
            cx.simulate_window_resize(handle, size(px(900.0), px(height)));
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let viewport = window.find("keychain-page-viewport").bounds();
                let region = window.find("keychain-empty-region");
                let content = window.find("empty-state-content");
                let region_bounds = region.bounds();
                let content_bounds = content.bounds();
                let actual = content_bounds.center();
                let expected = region_bounds.center();
                let tolerance = px(1.0 / window.scale_factor());

                assert!(region.visible() && content.visible());
                assert!(region_bounds.size.height > px(0.0));
                assert!(content_bounds.size.height > px(0.0));
                assert!(
                    (actual.x - expected.x).as_f32().abs() <= tolerance.as_f32()
                        && (actual.y - expected.y).as_f32().abs() <= tolerance.as_f32(),
                    "keychain empty content must center in the body below its tabs: \
                     content={content_bounds:?}, region={region_bounds:?}"
                );
                assert!(
                    region_bounds.top() > viewport.top()
                        && region_bounds.bottom() <= viewport.bottom() + tolerance,
                    "the tab header must retain its own height above the empty region"
                );
                assert!(
                    content_bounds.left() >= region_bounds.left() - tolerance
                        && content_bounds.right() <= region_bounds.right() + tolerance
                        && content_bounds.top() >= region_bounds.top() - tolerance
                        && content_bounds.bottom() <= region_bounds.bottom() + tolerance,
                    "the complete empty-state group must remain inside its visible body"
                );
                if height == 360.0 {
                    assert!(
                        region_bounds.size.height < px(420.0),
                        "the short viewport must exercise the former fixed minimum"
                    );
                }
            })
            .expect("measure keychain empty-page geometry");
        }
    }
}

#[gpui_kit::test]
fn managed_keys_empty_page_stays_centered_when_window_shrinks(cx: &mut TestAppContext) {
    assert_empty_keychain_page_geometry(cx, KeychainPageView::ManagedKeys);
}

#[gpui_kit::test]
fn agent_identities_empty_page_stays_centered_when_window_shrinks(cx: &mut TestAppContext) {
    assert_empty_keychain_page_geometry(cx, KeychainPageView::AgentIdentities);
}

#[gpui_kit::test]
fn retained_rename_surface_cannot_edit_replacement_and_failed_save_keeps_draft(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = ManagedKeyStore::with_path(directory.path().join("managed_keys.toml"));
    let known_hosts = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    let catalog = Rc::new(CatalogFixture::default());
    catalog.keys.replace(vec![key("alpha"), key("bravo")]);
    store.save(&catalog.keys.borrow()).unwrap();
    catalog.store.replace(Some(store.clone()));
    let source = catalog.clone();
    let key_store = store.clone();
    let runtime_handle = runtime.handle().clone();
    let (window_handle, view) = cx.update(|cx| {
        gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, move |window, cx| {
            cx.new(|cx| {
                let query_source = source.clone();
                let controller = cx.new(|cx| {
                    KeychainController::new(
                        KeychainControllerArgs {
                            managed_keys: source.keys.borrow().clone(),
                            runtime: runtime_handle,
                            keychain_store: Some(key_store),
                            known_hosts,
                            secrets: SecretStore::with_credentials(CredentialStore::with_backend(
                                "keychain-lifecycle-tests",
                                EmptyCredentials,
                            )),
                            local_vault_status: LocalVaultStatus::Disabled,
                            managed_key_query: ManagedKeyQueryPort::new(move |_| {
                                query_source.keys.borrow().clone()
                            }),
                            managed_key_commit: ManagedKeyCommitPort::new(source.clone()),
                        },
                        CatalogQueryPort::new(source.clone()),
                        window,
                        cx,
                    )
                });
                controller.update(cx, |owner, cx| {
                    owner.request_managed_key_rename("alpha", window, cx)
                });
                let snapshot = controller
                    .read(cx)
                    .overlay_snapshots(controller.clone(), cx)
                    .pop();
                Harness {
                    controller,
                    snapshot,
                }
            })
        })
        .expect("open keychain lifecycle test window")
    });
    let controller = view.read_with(cx, |view, _| view.controller.clone());
    let old_input = controller.read_with(cx, |owner, _| owner.forms.rename_name_input.clone());
    cx.update_window(window_handle, |_, window, cx| {
        controller.update(cx, |owner, cx| {
            owner.request_managed_key_rename("bravo", window, cx)
        })
    })
    .unwrap();
    let current_input = controller.read_with(cx, |owner, _| owner.forms.rename_name_input.clone());
    assert_ne!(old_input.entity_id(), current_input.entity_id());
    assert_eq!(old_input.read_with(cx, |input, _| input.value()), "alpha");
    assert_eq!(
        current_input.read_with(cx, |input, _| input.value()),
        "bravo"
    );
    cx.update_window(window_handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("managed-key-rename-confirm", cx);
        assert_eq!(
            controller
                .read(cx)
                .pending_managed_key_rename
                .as_ref()
                .unwrap()
                .key_id,
            "bravo"
        );
    })
    .unwrap();
    assert_eq!(catalog.commits.get(), 0);
    cx.update_window(window_handle, |_, _, cx| {
        view.update(cx, |view, cx| {
            view.snapshot = view
                .controller
                .read(cx)
                .overlay_snapshots(view.controller.clone(), cx)
                .pop();
            cx.notify();
        })
    })
    .unwrap();
    catalog.fail.set(true);
    cx.update_window(window_handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-a", cx);
        window.input("Bravo 中文", cx);
        assert_eq!(current_input.read(cx).value(), "Bravo 中文");
        window.click("managed-key-rename-confirm", cx);
        assert_eq!(
            controller
                .read(cx)
                .pending_managed_key_rename
                .as_ref()
                .unwrap()
                .key_id,
            "bravo"
        );
        assert_eq!(current_input.read(cx).value(), "Bravo 中文");
    })
    .unwrap();
    assert_eq!(catalog.commits.get(), 0);
    assert_eq!(store.load().unwrap(), vec![key("alpha"), key("bravo")]);
    catalog.keys.borrow_mut().push(key("charlie"));
    store.save(&catalog.keys.borrow()).unwrap();
    catalog.fail.set(false);
    cx.update_window(window_handle, |_, window, cx| {
        window.click("managed-key-rename-confirm", cx)
    })
    .unwrap();
    assert_eq!(catalog.commits.get(), 1);
    assert_eq!(catalog.keys.borrow()[1].name, "Bravo 中文");
    assert_eq!(catalog.keys.borrow()[2].id, "charlie");
    assert_eq!(store.load().unwrap(), *catalog.keys.borrow());
    assert!(controller.read_with(cx, |owner, _| owner.pending_managed_key_rename.is_none()));
}
