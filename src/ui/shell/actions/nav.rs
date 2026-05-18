use super::super::*;
use crate::ui::i18n;

impl AppView {
    pub(in crate::ui::shell) fn open_hosts_tab(&mut self, cx: &mut Context<Self>) {
        let previous_active_terminal_tab_id = self
            .workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(|tab| {
                tab.as_session()
                    .filter(|session| session.purpose == SessionPurpose::Terminal)
                    .map(|_| tab.id)
            });
        self.unload_active_topbar_workspace(cx);

        let tab_id = self.workspace_state.next_tab_id;
        self.workspace_state.next_tab_id += 1;

        self.workspace_state.tabs.push(TabState::new_hosts(tab_id));
        self.workspace_state.active_topbar_tab = Some(self.workspace_state.tabs.len() - 1);
        self.workspace_state.workspace.active_tab = None;
        self.panel_view.sidebar_section = SidebarSection::Hosts;
        self.editors.host_editor_open = false;
        self.editors.host_editor_is_new = false;
        if let Some(terminal_tab_id) = previous_active_terminal_tab_id {
            self.start_hosts_to_terminal_transition(
                tab_id,
                terminal_tab_id,
                HostsToTerminalTransitionDirection::ToHosts,
                false,
            );
            self.workspace_state.terminal_view_transition = None;
            self.workspace_state.visible_terminal_view_tab_id = None;
        } else {
            self.workspace_state.hosts_to_terminal_transition = None;
        }
        self.status_message = i18n::string("navigation.messages.opened_new_hosts_tab");
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_sidebar_section(
        &mut self,
        section: SidebarSection,
        cx: &mut Context<Self>,
    ) {
        let preserve_hosts_tab_selection = self.active_tab_is_hosts();

        if self.panel_view.sidebar_section == section
            && (self.workspace_state.active_topbar_tab.is_none() || preserve_hosts_tab_selection)
        {
            return;
        }

        self.unload_active_topbar_workspace(cx);
        self.panel_view.sidebar_section = section;
        if !preserve_hosts_tab_selection {
            self.workspace_state.active_topbar_tab = None;
        }
        self.workspace_state.workspace.active_tab = None;
        if section != SidebarSection::PortForwarding {
            self.editors.port_forward_editor_open = false;
            self.editors.port_forward_editor_profile_id = None;
            self.editors.port_forward_editor_rule_id = None;
        }
        if section != SidebarSection::Snippets {
            self.editors.snippets_editor_open = false;
        }
        if section != SidebarSection::Keychain {
            self.editors.keychain_editor_open = false;
        }
        if section == SidebarSection::Keychain {
            self.refresh_keychain_data(cx);
        } else {
            let title = section.title();
            self.status_message = i18n::string_args(
                "navigation.messages.viewing_section",
                &[("section", &title)],
            );
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn open_add_host_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_profile(window, cx);
    }

    pub(in crate::ui::shell) fn open_host_editor(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_profile(index, window, cx);
        self.editors.host_editor_open = true;
        self.editors.host_editor_is_new = false;
        let name = self.data.sessions[index].name.clone();
        self.status_message =
            i18n::string_args("navigation.messages.editing_profile", &[("name", &name)]);
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_host_editor(&mut self, cx: &mut Context<Self>) {
        if !self.editors.host_editor_open {
            return;
        }

        self.editors.host_editor_open = false;
        self.editors.host_editor_is_new = false;
        cx.notify();
    }
}
