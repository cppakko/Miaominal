use super::super::super::super::super::*;

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
    local: bool,
}

impl HostEditorKindLayout {
    pub(super) fn for_kind(kind: ProfileKind) -> Self {
        Self {
            local: kind == ProfileKind::Local,
        }
    }

    pub(super) fn is_local(self) -> bool {
        self.local
    }

    pub(super) fn shows_address_fields(self) -> bool {
        !self.local
    }

    pub(super) fn shows_credentials(self) -> bool {
        !self.local
    }

    pub(super) fn shows_ssh_advanced_fields(self) -> bool {
        !self.local
    }

    pub(super) fn shows_shell_type(self) -> bool {
        !self.local
    }

    pub(super) fn shows_group(self) -> bool {
        !self.local
    }

    pub(super) fn shows_terminal_section(self) -> bool {
        self.local
    }

    pub(super) fn shows_test_connection(self) -> bool {
        !self.local
    }

    pub(super) fn title_key(self, is_new: bool) -> &'static str {
        match (self.local, is_new) {
            (false, true) => "hosts.editor.titles.add",
            (false, false) => "hosts.editor.titles.edit",
            (true, true) => "hosts.editor.titles.add_local",
            (true, false) => "hosts.editor.titles.edit_local",
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
        assert!(layout.shows_ssh_advanced_fields());
        assert!(layout.shows_shell_type());
        assert!(layout.shows_group());
        assert!(layout.shows_test_connection());
        assert!(!layout.shows_terminal_section());
    }

    #[test]
    fn local_layout_hides_ssh_only_sections_and_reveals_terminal_section() {
        let layout = HostEditorKindLayout::for_kind(ProfileKind::Local);

        assert!(layout.is_local());
        assert!(!layout.shows_address_fields());
        assert!(!layout.shows_credentials());
        assert!(!layout.shows_ssh_advanced_fields());
        assert!(!layout.shows_shell_type());
        assert!(!layout.shows_group());
        assert!(!layout.shows_test_connection());
        assert!(layout.shows_terminal_section());
    }

    #[test]
    fn editor_title_key_follows_kind_and_creation_state() {
        let ssh = HostEditorKindLayout::for_kind(ProfileKind::Ssh);
        let local = HostEditorKindLayout::for_kind(ProfileKind::Local);

        assert_eq!(ssh.title_key(true), "hosts.editor.titles.add");
        assert_eq!(ssh.title_key(false), "hosts.editor.titles.edit");
        assert_eq!(local.title_key(true), "hosts.editor.titles.add_local");
        assert_eq!(local.title_key(false), "hosts.editor.titles.edit_local");
    }
}
