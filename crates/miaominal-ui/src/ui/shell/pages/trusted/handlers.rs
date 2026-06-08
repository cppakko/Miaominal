use super::super::super::*;
use crate::ui::i18n;

impl AppView {
    pub(in crate::ui::shell) fn handle_trusted_host_filter_change(
        &mut self,
        filter: TrustedHostFilter,
        cx: &mut Context<Self>,
    ) {
        self.panel_view.trusted_host_filter = filter;
        cx.notify();
    }

    pub(in crate::ui::shell) fn select_trusted_known_host(
        &mut self,
        host: String,
        port: u16,
        fingerprint: String,
        cx: &mut Context<Self>,
    ) {
        self.panels.selected_known_host = Some((host, port, fingerprint));
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_trusted_known_host_sidebar(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if self.panels.selected_known_host.take().is_some() {
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn copy_known_host_fingerprint(
        &mut self,
        fingerprint: String,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(fingerprint));
        self.status_message = i18n::string("trusted.messages.copied_fingerprint");
        cx.notify();
    }

    pub(in crate::ui::shell) fn copy_known_host_address(
        &mut self,
        host: String,
        port: u16,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(format!("{host}:{port}")));
        self.status_message = i18n::string("trusted.messages.copied_address");
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_linked_profile_from_known_host(
        &mut self,
        profile_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .data
            .sessions
            .iter()
            .position(|profile| profile.id == profile_id)
        else {
            self.status_message = i18n::string("trusted.messages.profile_not_found");
            cx.notify();
            return;
        };

        self.panel_view.sidebar_section = SidebarSection::Hosts;
        self.open_host_editor(index, window, cx);
    }

    pub(in crate::ui::shell) fn request_trusted_known_host_removal(
        &mut self,
        host: String,
        port: u16,
        cx: &mut Context<Self>,
    ) {
        if !self
            .data
            .known_hosts_entries
            .iter()
            .any(|entry| entry.host.as_str() == host.as_str() && entry.port == port)
        {
            let port_text = port.to_string();
            self.status_message = i18n::string_args(
                "session.messages.no_host_key_entry",
                &[("host", &host), ("port", &port_text)],
            );
            cx.notify();
            return;
        }

        self.dialogs.pending_known_host_delete = Some(PendingKnownHostDeleteState { host, port });
        cx.notify();
    }

    pub(in crate::ui::shell) fn confirm_trusted_known_host_removal(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.dialogs.pending_known_host_delete.take() else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::KnownHostDelete(pending.clone()), cx);

        if self
            .panels
            .selected_known_host
            .as_ref()
            .is_some_and(|(host, port, _)| host == &pending.host && *port == pending.port)
        {
            self.panels.selected_known_host = None;
        }

        self.remove_known_host(pending.host, pending.port, cx);
    }

    pub(in crate::ui::shell) fn cancel_trusted_known_host_removal(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if let Some(pending) = self.dialogs.pending_known_host_delete.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::KnownHostDelete(pending), cx);
        }
    }

    pub(in crate::ui::shell) fn handle_trusted_host_key_decision(
        &mut self,
        decision: HostKeyDecision,
        cx: &mut Context<Self>,
    ) {
        self.resolve_host_key_prompt(decision, cx);
    }
}
