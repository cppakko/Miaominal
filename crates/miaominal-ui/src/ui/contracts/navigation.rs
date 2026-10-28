use crate::ui::assets::AppIcon;
use crate::ui::i18n;
use gpui_kit::component::{Icon, Sizable as _};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(in crate::ui) enum SidebarSection {
    #[default]
    Hosts,
    Keychain,
    PortForwarding,
    Snippets,
    KnownHosts,
    Settings,
}

impl SidebarSection {
    pub(in crate::ui) fn all() -> [Self; 5] {
        [
            Self::Hosts,
            Self::Keychain,
            Self::PortForwarding,
            Self::Snippets,
            Self::KnownHosts,
        ]
    }

    pub(in crate::ui) fn title(self) -> String {
        i18n::string(match self {
            Self::Hosts => "navigation.section.hosts.title",
            Self::Keychain => "navigation.section.keychain.title",
            Self::PortForwarding => "navigation.section.forwarding.title",
            Self::Snippets => "navigation.section.snippets.title",
            Self::KnownHosts => "navigation.section.known_hosts.title",
            Self::Settings => "navigation.section.settings.title",
        })
    }

    pub(in crate::ui) fn icon(self) -> Icon {
        match self {
            Self::Hosts => Icon::new(AppIcon::Computer).large(),
            Self::Keychain => Icon::new(AppIcon::Key).large(),
            Self::PortForwarding => Icon::new(AppIcon::Forward).large(),
            Self::Snippets => Icon::new(AppIcon::Notebook).large(),
            Self::KnownHosts => Icon::new(AppIcon::FingerPrint).large(),
            Self::Settings => Icon::new(AppIcon::Settings).large(),
        }
    }

    pub(in crate::ui) fn subtitle(self) -> String {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum PageEditorSidebarKind {
    Hosts,
    PortForwarding,
    Snippets,
    Keychain,
    KnownHosts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum SettingsDestination {
    SshBridgeSecurity,
    Sync,
    SearchItem(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum ProfileViewMode {
    Grid,
    List,
}
