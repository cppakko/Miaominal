use super::super::super::*;
use crate::ui::i18n;

impl AppView {
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
