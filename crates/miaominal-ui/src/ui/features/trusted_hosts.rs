use crate::ui::contracts::CatalogQueryPort;
use crate::ui::contracts::commands::{AppCommand, TabOpenRequest};
use crate::ui::contracts::navigation::PageEditorSidebarKind;
use crate::ui::contracts::overlay::{OverlayRevisionState, OverlaySnapshot};
use crate::ui::i18n;
#[cfg(test)]
use gpui_kit::Subscription;
#[cfg(test)]
use gpui_kit::div;
#[cfg(test)]
use gpui_kit::prelude::*;
use gpui_kit::{ClipboardItem, EventEmitter, FocusHandle};
use gpui_kit::{Context, Entity, Window};
use miaominal_core::known_host::KnownHostEntry;
#[cfg(test)]
use miaominal_core::profile::SessionProfile;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use std::cell::{Ref, RefCell};

#[path = "trusted_hosts/view/mod.rs"]
mod view;

#[derive(Debug, Clone)]
pub(in crate::ui) struct PendingKnownHostDeleteState {
    pub(in crate::ui) host: String,
    pub(in crate::ui) port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui) enum TrustedHostFilter {
    #[default]
    All,
    Linked,
    Orphaned,
    DefaultPort,
    CustomPort,
}

pub(in crate::ui) struct TrustedHostsController {
    store: KnownHostsStore,
    catalog: CatalogQueryPort,
    known_hosts_entries: RefCell<Vec<KnownHostEntry>>,
    filter: TrustedHostFilter,
    selected: RefCell<Option<(String, u16, String)>>,
    details_focus: FocusHandle,
    pending_delete: RefCell<Option<PendingKnownHostDeleteState>>,
    overlay_revisions: RefCell<OverlayRevisionState>,
}

impl EventEmitter<AppCommand> for TrustedHostsController {}

impl TrustedHostsController {
    pub(in crate::ui) fn new(
        store: KnownHostsStore,
        entries: Vec<KnownHostEntry>,
        catalog: CatalogQueryPort,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            store,
            catalog,
            known_hosts_entries: RefCell::new(entries),
            filter: TrustedHostFilter::All,
            selected: RefCell::new(None),
            details_focus: cx.focus_handle(),
            pending_delete: RefCell::new(None),
            overlay_revisions: RefCell::default(),
        }
    }

    pub(in crate::ui) fn search_snapshot(&self) -> Vec<KnownHostEntry> {
        self.known_hosts_entries.borrow().clone()
    }

    pub(in crate::ui) fn focus_details(&self, window: &mut Window, cx: &mut gpui_kit::App) {
        window.focus(&self.details_focus, cx);
    }

    fn known_hosts_entries(&self) -> Ref<'_, Vec<KnownHostEntry>> {
        self.known_hosts_entries.borrow()
    }

    pub(in crate::ui) fn replace_known_hosts_entries(&self, entries: Vec<KnownHostEntry>) {
        *self.known_hosts_entries.borrow_mut() = entries;
    }

    pub(in crate::ui) fn refresh_known_hosts_with_feedback(&self, cx: &mut Context<Self>) {
        match self.store.list() {
            Ok(entries) => {
                if *self.known_hosts_entries.borrow() != entries {
                    self.replace_known_hosts_entries(entries.clone());
                    cx.emit(AppCommand::KnownHostsChanged);
                }
                cx.emit(AppCommand::Feedback(i18n::string(
                    "trusted.messages.refreshed",
                )));
            }
            Err(error) => log::warn!("failed to refresh known_hosts list: {error:?}"),
        }
        cx.notify();
    }

    pub(in crate::ui) fn select_trusted_known_host(
        &self,
        host: String,
        port: u16,
        fingerprint: String,
        cx: &mut Context<Self>,
    ) {
        self.set_selected_known_host(Some((host, port, fingerprint)));
        cx.emit(AppCommand::SidebarEditorStateChanged(Some(
            PageEditorSidebarKind::KnownHosts,
        )));
        cx.notify();
    }

    pub(in crate::ui) fn close_trusted_known_host_sidebar(&self, cx: &mut Context<Self>) {
        if self.take_selected_known_host().is_some() {
            cx.emit(AppCommand::SidebarEditorStateChanged(None));
            cx.notify();
        }
    }

    pub(in crate::ui) fn copy_known_host_fingerprint(
        &self,
        fingerprint: String,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(fingerprint));
        cx.emit(AppCommand::Feedback(i18n::string(
            "trusted.messages.copied_fingerprint",
        )));
    }

    pub(in crate::ui) fn copy_known_host_address(
        &self,
        host: String,
        port: u16,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(format!("{host}:{port}")));
        cx.emit(AppCommand::Feedback(i18n::string(
            "trusted.messages.copied_address",
        )));
    }

    pub(in crate::ui) fn open_linked_profile_from_known_host(
        &self,
        profile_id: String,
        cx: &mut Context<Self>,
    ) {
        cx.emit(AppCommand::OpenTab(TabOpenRequest::ProfileEditor {
            profile_id,
            open_hosts_tab: false,
        }));
    }

    pub(in crate::ui) fn request_trusted_known_host_removal(
        &self,
        host: String,
        port: u16,
        cx: &mut Context<Self>,
    ) {
        if !self
            .known_hosts_entries
            .borrow()
            .iter()
            .any(|entry| entry.host == host && entry.port == port)
        {
            let port_text = port.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "session.messages.no_host_key_entry",
                &[("host", &host), ("port", &port_text)],
            )));
            return;
        }

        self.set_pending_known_host_delete(Some(PendingKnownHostDeleteState { host, port }));
        cx.notify();
    }

    pub(in crate::ui) fn confirm_trusted_known_host_removal(&self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_known_host_delete() else {
            return;
        };
        let snapshot = self.known_host_delete_overlay(cx.entity());
        let result = self.store.remove(&pending.host, pending.port);
        let message = match result {
            Ok(removed) => {
                self.take_pending_known_host_delete();
                if let Some(snapshot) = snapshot {
                    cx.emit(AppCommand::OverlayDismissed(snapshot));
                }
                if self
                    .selected_known_host()
                    .as_ref()
                    .is_some_and(|(host, port, _)| host == &pending.host && *port == pending.port)
                {
                    self.set_selected_known_host(None);
                    cx.emit(AppCommand::SidebarEditorStateChanged(None));
                }
                // The successful store operation is authoritative even if a subsequent read fails.
                let mut entries = self.known_hosts_entries.borrow().clone();
                entries.retain(|entry| entry.host != pending.host || entry.port != pending.port);
                let entries = self.store.list().unwrap_or(entries);
                self.replace_known_hosts_entries(entries.clone());
                cx.emit(AppCommand::KnownHostsChanged);
                let port = pending.port.to_string();
                i18n::string_args(
                    if removed {
                        "session.messages.removed_host_key"
                    } else {
                        "session.messages.no_host_key_entry"
                    },
                    &[("host", &pending.host), ("port", &port)],
                )
            }
            Err(error) => i18n::string_args(
                "session.messages.remove_host_key_failed",
                &[("error", &error.to_string())],
            ),
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui) fn cancel_trusted_known_host_removal(&self, cx: &mut Context<Self>) {
        let snapshot = self.known_host_delete_overlay(cx.entity());
        if self.take_pending_known_host_delete().is_some()
            && let Some(snapshot) = snapshot
        {
            cx.emit(AppCommand::OverlayDismissed(snapshot));
        }
    }

    pub(in crate::ui) fn selected_known_host(&self) -> Option<(String, u16, String)> {
        self.selected.borrow().clone()
    }

    pub(in crate::ui) fn set_selected_known_host(&self, selected: Option<(String, u16, String)>) {
        *self.selected.borrow_mut() = selected;
    }

    pub(in crate::ui) fn take_selected_known_host(&self) -> Option<(String, u16, String)> {
        self.selected.take()
    }

    pub(in crate::ui) fn pending_known_host_delete(&self) -> Option<PendingKnownHostDeleteState> {
        self.pending_delete.borrow().clone()
    }

    pub(in crate::ui) fn set_pending_known_host_delete(
        &self,
        pending: Option<PendingKnownHostDeleteState>,
    ) {
        self.overlay_revisions
            .borrow_mut()
            .begin("known-host-delete");
        *self.pending_delete.borrow_mut() = pending;
    }

    pub(in crate::ui) fn take_pending_known_host_delete(
        &self,
    ) -> Option<PendingKnownHostDeleteState> {
        self.pending_delete.take()
    }

    pub(in crate::ui) fn set_trusted_host_filter(
        &mut self,
        filter: TrustedHostFilter,
        cx: &mut Context<Self>,
    ) {
        self.filter = filter;
        cx.notify();
    }
}

impl TrustedHostsController {
    pub(in crate::ui) fn current_overlay(
        &self,
        controller: Entity<Self>,
    ) -> Option<OverlaySnapshot> {
        self.known_host_delete_overlay(controller)
    }

    fn known_host_delete_overlay(&self, controller: Entity<Self>) -> Option<OverlaySnapshot> {
        let prompt = self.pending_known_host_delete()?;
        let revision = self.overlay_revisions.borrow().current("known-host-delete");
        let render_controller = controller.clone();
        let dismiss_controller = controller.clone();
        Some(
            OverlaySnapshot::new(
                format!("trusted-hosts:delete:{}:{}", prompt.host, prompt.port),
                revision,
                move |progress, _, _| {
                    Self::render_trusted_known_host_delete_prompt(
                        render_controller.clone(),
                        &prompt,
                        revision,
                        progress,
                    )
                },
                move |_, cx| {
                    dismiss_controller.update(cx, |this, cx| {
                        if this.overlay_revisions.borrow().current("known-host-delete") == revision
                        {
                            this.cancel_trusted_known_host_removal(cx);
                        }
                    });
                },
            )
            .with_submit(move |_, cx| {
                controller.update(cx, |this, cx| {
                    if this.overlay_revisions.borrow().current("known-host-delete") == revision {
                        this.confirm_trusted_known_host_removal(cx);
                    }
                });
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contracts::CatalogReader;
    use gpui_kit::{Render, TestAppContext};
    use std::rc::Rc;

    struct EmptyCatalog;
    impl CatalogReader for EmptyCatalog {
        fn profiles(&self) -> Vec<SessionProfile> {
            vec![]
        }
        fn profile(&self, _: &str) -> Option<SessionProfile> {
            None
        }
        fn proxies(&self) -> Vec<miaominal_core::proxy::ProxyProfile> {
            vec![]
        }
    }

    struct Harness {
        controller: Entity<TrustedHostsController>,
        commits: Vec<Vec<KnownHostEntry>>,
        _subscription: Subscription,
    }
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui_kit::test]
    fn deletion_failure_retains_target_and_snapshot_adoption_does_not_publish(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let path = std::env::temp_dir().join(format!(
            "miaominal-trusted-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "example.test ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\nother.test ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n").unwrap();
        let store = KnownHostsStore::with_path(path.clone());
        let entries = store.list().unwrap();
        assert_eq!(entries.len(), 2);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let controller = cx.new(|cx| {
                TrustedHostsController::new(
                    KnownHostsStore::with_path(std::env::temp_dir()),
                    entries.clone(),
                    CatalogQueryPort::new(Rc::new(EmptyCatalog)),
                    window,
                    cx,
                )
            });
            let subscription = cx.subscribe(
                &controller,
                |this: &mut Harness, controller, event: &AppCommand, cx| {
                    if let AppCommand::KnownHostsChanged = event {
                        this.commits
                            .push(controller.read(cx).known_hosts_entries().clone());
                    }
                },
            );
            Harness {
                controller,
                commits: vec![],
                _subscription: subscription,
            }
        });
        let controller = view.read_with(cx, |view, _| view.controller.clone());
        cx.update(|_, cx| {
            controller.update(cx, |controller, cx| {
                controller.select_trusted_known_host(
                    entries[0].host.clone(),
                    entries[0].port,
                    entries[0].fingerprint.clone(),
                    cx,
                );
                controller.request_trusted_known_host_removal(
                    entries[0].host.clone(),
                    entries[0].port,
                    cx,
                );
                controller.confirm_trusted_known_host_removal(cx);
                assert!(controller.pending_known_host_delete().is_some());
                assert!(controller.selected_known_host().is_some());
                assert_eq!(*controller.known_hosts_entries.borrow(), entries);
                controller.refresh_known_hosts_with_feedback(cx);
                assert_eq!(*controller.known_hosts_entries.borrow(), entries);
                let mut reordered = entries.clone();
                reordered.reverse();
                controller.replace_known_hosts_entries(reordered);
                cx.notify();
            })
        });
        assert!(view.read_with(cx, |view, _| view.commits.is_empty()));
        cx.update(|_, cx| {
            controller.update(cx, |controller, cx| {
                controller.store = store.clone();
                controller.confirm_trusted_known_host_removal(cx);
                assert!(controller.pending_known_host_delete().is_none());
                assert!(controller.selected_known_host().is_none());
                assert_eq!(
                    controller.known_hosts_entries.borrow().as_slice(),
                    &entries[1..]
                );
            })
        });
        assert_eq!(
            view.read_with(cx, |view, _| view.commits.clone()),
            vec![entries[1..].to_vec()]
        );
        assert_eq!(store.list().unwrap(), entries[1..]);
        cx.update(|_, cx| {
            controller.update(cx, |controller, cx| {
                controller.replace_known_hosts_entries(entries[1..].to_vec());
                controller.refresh_known_hosts_with_feedback(cx);
                cx.notify();
            })
        });
        assert_eq!(view.read_with(cx, |view, _| view.commits.len()), 1);
        std::fs::remove_file(path).unwrap();
    }
}
