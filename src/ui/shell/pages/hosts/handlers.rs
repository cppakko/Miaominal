use super::super::super::*;
use crate::ui::i18n;

impl AppView {
    pub(in crate::ui::shell) fn handle_hosts_group_filter_toggle(
        &mut self,
        group: String,
        cx: &mut Context<Self>,
    ) {
        if self.panel_view.hosts_group_filter.as_deref() == Some(group.as_str()) {
            self.panel_view.hosts_group_filter = None;
            self.status_message = i18n::string("hosts.messages.cleared_group_filter");
        } else {
            self.panel_view.hosts_group_filter = Some(group.clone());
            self.status_message =
                i18n::string_args("hosts.messages.filtering_by_group", &[("group", &group)]);
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn handle_hosts_view_mode_change(
        &mut self,
        mode: ProfileViewMode,
        cx: &mut Context<Self>,
    ) {
        self.panel_view.hosts_view_mode = mode;
        cx.notify();
    }
}
