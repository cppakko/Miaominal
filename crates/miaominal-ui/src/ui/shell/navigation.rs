use super::*;
use crate::ui::i18n;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(in crate::ui::shell) enum SidebarSection {
    #[default]
    Hosts,
    Keychain,
    PortForwarding,
    Snippets,
    KnownHosts,
    Settings,
}

impl SidebarSection {
    pub(in crate::ui::shell) fn all() -> [Self; 5] {
        [
            Self::Hosts,
            Self::Keychain,
            Self::PortForwarding,
            Self::Snippets,
            Self::KnownHosts,
        ]
    }

    pub(in crate::ui::shell) fn title(self) -> String {
        i18n::string(match self {
            Self::Hosts => "navigation.section.hosts.title",
            Self::Keychain => "navigation.section.keychain.title",
            Self::PortForwarding => "navigation.section.forwarding.title",
            Self::Snippets => "navigation.section.snippets.title",
            Self::KnownHosts => "navigation.section.known_hosts.title",
            Self::Settings => "navigation.section.settings.title",
        })
    }

    pub(in crate::ui::shell) fn icon(self) -> Icon {
        match self {
            Self::Hosts => Icon::new(AppIcon::Computer).large(),
            Self::Keychain => Icon::new(AppIcon::Key).large(),
            Self::PortForwarding => Icon::new(AppIcon::Forward).large(),
            Self::Snippets => Icon::new(AppIcon::Notebook).large(),
            Self::KnownHosts => Icon::new(AppIcon::FingerPrint).large(),
            Self::Settings => Icon::new(AppIcon::Settings).large(),
        }
    }

    pub(in crate::ui::shell) fn subtitle(self) -> String {
        i18n::string(match self {
            Self::Hosts => "navigation.section.hosts.subtitle",
            Self::Keychain => "navigation.section.keychain.subtitle",
            Self::PortForwarding => "navigation.section.forwarding.subtitle",
            Self::Snippets => "navigation.section.snippets.subtitle",
            Self::KnownHosts => "navigation.section.known_hosts.subtitle",
            Self::Settings => "navigation.section.settings.subtitle",
        })
    }
}
