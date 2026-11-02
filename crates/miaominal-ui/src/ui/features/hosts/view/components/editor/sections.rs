use gpui_kit::component::stepper::StepperItem;
use gpui_kit::component::{IconName, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{SharedString, div, rgb};
use miaominal_core::profile::ProfileKind;

pub(super) fn proxy_jump_stepper_item(
    icon: IconName,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
) -> StepperItem {
    let roles = miaominal_settings::current_theme().material.roles;

    StepperItem::new().icon(icon).child(
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Input.scaled())
                    .text_color(rgb(roles.on_surface))
                    .child(title.into()),
            )
            .child(
                div()
                    .text_size(miaominal_settings::FontSize::Body.scaled())
                    .text_color(rgb(roles.on_surface_variant))
                    .child(detail.into()),
            ),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct HostEditorKindLayout {
    kind: ProfileKind,
}

impl HostEditorKindLayout {
    pub(super) fn for_kind(kind: ProfileKind) -> Self {
        Self { kind }
    }

    fn ssh(self) -> bool {
        self.kind == ProfileKind::Ssh
    }

    pub(super) fn is_local(self) -> bool {
        self.kind == ProfileKind::Local
    }

    pub(super) fn is_telnet(self) -> bool {
        self.kind == ProfileKind::Telnet
    }

    /// Index of this kind in the editor's kind switch.
    pub(super) fn kind_index(self) -> usize {
        match self.kind {
            ProfileKind::Ssh | ProfileKind::Rdp => 0,
            ProfileKind::Telnet => 1,
            ProfileKind::Local => 2,
        }
    }

    pub(super) fn kind_for_index(index: usize) -> Option<ProfileKind> {
        match index {
            0 => Some(ProfileKind::Ssh),
            1 => Some(ProfileKind::Telnet),
            2 => Some(ProfileKind::Local),
            _ => None,
        }
    }

    pub(super) fn shows_address_fields(self) -> bool {
        !self.is_local()
    }

    pub(super) fn port_label_key(self) -> &'static str {
        if self.is_telnet() {
            "hosts.editor.fields.telnet_port"
        } else {
            "hosts.editor.fields.ssh_port"
        }
    }

    pub(super) fn shows_credentials(self) -> bool {
        !self.is_local()
    }

    /// SSH identity sources; Telnet only has an optional auto-login password.
    pub(super) fn shows_auth_method(self) -> bool {
        self.ssh()
    }

    pub(super) fn shows_agent_forwarding(self) -> bool {
        self.ssh()
    }

    /// Startup command and entry proxy, shared by SSH and Telnet.
    pub(super) fn shows_remote_advanced_fields(self) -> bool {
        !self.is_local()
    }

    pub(super) fn shows_proxy_jump(self) -> bool {
        self.ssh()
    }

    pub(super) fn shows_environment_variables(self) -> bool {
        !self.is_telnet()
    }

    pub(super) fn shows_shell_type(self) -> bool {
        self.ssh()
    }

    pub(super) fn shows_group(self) -> bool {
        !self.is_local()
    }

    pub(super) fn shows_terminal_section(self) -> bool {
        self.is_local()
    }

    pub(super) fn shows_test_connection(self) -> bool {
        self.ssh()
    }

    pub(super) fn title_key(self, is_new: bool) -> &'static str {
        match (self.kind, is_new) {
            (ProfileKind::Local, true) => "hosts.editor.titles.add_local",
            (ProfileKind::Local, false) => "hosts.editor.titles.edit_local",
            (ProfileKind::Telnet, true) => "hosts.editor.titles.add_telnet",
            (ProfileKind::Telnet, false) => "hosts.editor.titles.edit_telnet",
            (_, true) => "hosts.editor.titles.add",
            (_, false) => "hosts.editor.titles.edit",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_layout_keeps_ssh_only_sections() {
        let layout = HostEditorKindLayout::for_kind(ProfileKind::Ssh);

        assert!(!layout.is_local());
        assert!(layout.shows_address_fields());
        assert!(layout.shows_credentials());
        assert!(layout.shows_auth_method());
        assert!(layout.shows_agent_forwarding());
        assert!(layout.shows_remote_advanced_fields());
        assert!(layout.shows_proxy_jump());
        assert!(layout.shows_environment_variables());
        assert!(layout.shows_shell_type());
        assert!(layout.shows_group());
        assert!(layout.shows_test_connection());
        assert!(!layout.shows_terminal_section());
        assert_eq!(layout.port_label_key(), "hosts.editor.fields.ssh_port");
    }

    #[test]
    fn local_layout_hides_ssh_only_sections_and_reveals_terminal_section() {
        let layout = HostEditorKindLayout::for_kind(ProfileKind::Local);

        assert!(layout.is_local());
        assert!(!layout.shows_address_fields());
        assert!(!layout.shows_credentials());
        assert!(!layout.shows_agent_forwarding());
        assert!(!layout.shows_remote_advanced_fields());
        assert!(!layout.shows_proxy_jump());
        assert!(layout.shows_environment_variables());
        assert!(!layout.shows_shell_type());
        assert!(!layout.shows_group());
        assert!(!layout.shows_test_connection());
        assert!(layout.shows_terminal_section());
    }

    #[test]
    fn telnet_layout_keeps_address_and_login_but_drops_ssh_features() {
        let layout = HostEditorKindLayout::for_kind(ProfileKind::Telnet);

        assert!(layout.is_telnet());
        assert!(layout.shows_address_fields());
        assert!(layout.shows_group());
        assert!(layout.shows_credentials());
        assert!(!layout.shows_auth_method());
        assert!(!layout.shows_agent_forwarding());
        assert!(layout.shows_remote_advanced_fields());
        assert!(!layout.shows_proxy_jump());
        assert!(!layout.shows_environment_variables());
        assert!(!layout.shows_shell_type());
        assert!(!layout.shows_test_connection());
        assert!(!layout.shows_terminal_section());
        assert_eq!(layout.port_label_key(), "hosts.editor.fields.telnet_port");
    }

    #[test]
    fn kind_switch_indices_round_trip() {
        for kind in [ProfileKind::Ssh, ProfileKind::Telnet, ProfileKind::Local] {
            let index = HostEditorKindLayout::for_kind(kind).kind_index();
            assert_eq!(HostEditorKindLayout::kind_for_index(index), Some(kind));
        }
        assert_eq!(HostEditorKindLayout::kind_for_index(3), None);
    }

    #[test]
    fn editor_title_key_follows_kind_and_creation_state() {
        let ssh = HostEditorKindLayout::for_kind(ProfileKind::Ssh);
        let local = HostEditorKindLayout::for_kind(ProfileKind::Local);
        let telnet = HostEditorKindLayout::for_kind(ProfileKind::Telnet);

        assert_eq!(ssh.title_key(true), "hosts.editor.titles.add");
        assert_eq!(ssh.title_key(false), "hosts.editor.titles.edit");
        assert_eq!(local.title_key(true), "hosts.editor.titles.add_local");
        assert_eq!(local.title_key(false), "hosts.editor.titles.edit_local");
        assert_eq!(telnet.title_key(true), "hosts.editor.titles.add_telnet");
        assert_eq!(telnet.title_key(false), "hosts.editor.titles.edit_telnet");
    }
}
