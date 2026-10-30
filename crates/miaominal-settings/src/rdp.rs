use serde::{Deserialize, Serialize};

/// External desktop clients supported by Miaominal. This preference is local to the device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RdpClientKind {
    #[default]
    Auto,
    Mstsc,
    WindowsApp,
    Remmina,
    SdlFreeRdp,
}

/// Device-local client selection; deliberately absent from `SyncedSettings`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "StoredRdpClientSettings")]
pub struct RdpClientSettings {
    kind: RdpClientKind,
    path: String,
}

/// Tolerant on-disk form: a client kind written by a newer build (or by hand) must not
/// make the whole settings file unreadable. Unknown kinds fall back to automatic
/// detection, and their path is discarded because it belongs to that unknown client.
#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StoredRdpClientKind {
    #[default]
    Auto,
    Mstsc,
    WindowsApp,
    Remmina,
    SdlFreeRdp,
    #[serde(other)]
    Unknown,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct StoredRdpClientSettings {
    kind: StoredRdpClientKind,
    path: String,
}

impl From<StoredRdpClientSettings> for RdpClientSettings {
    fn from(stored: StoredRdpClientSettings) -> Self {
        let kind = match stored.kind {
            StoredRdpClientKind::Auto => RdpClientKind::Auto,
            StoredRdpClientKind::Mstsc => RdpClientKind::Mstsc,
            StoredRdpClientKind::WindowsApp => RdpClientKind::WindowsApp,
            StoredRdpClientKind::Remmina => RdpClientKind::Remmina,
            StoredRdpClientKind::SdlFreeRdp => RdpClientKind::SdlFreeRdp,
            StoredRdpClientKind::Unknown => return Self::default(),
        };
        Self::new(kind, stored.path)
    }
}

impl RdpClientSettings {
    pub fn new(kind: RdpClientKind, path: impl Into<String>) -> Self {
        Self {
            kind,
            path: path.into().trim().to_string(),
        }
    }

    pub fn kind(&self) -> RdpClientKind {
        self.kind
    }
    pub fn path(&self) -> &str {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppSettings, changed};

    #[test]
    fn client_settings_are_local_and_survive_sync() {
        let original = AppSettings::default();
        let mut local = original.clone();
        local.rdp_client =
            RdpClientSettings::new(RdpClientKind::Mstsc, "C:/custom client/mstsc.exe");
        assert!(changed(&original, &local));
        assert_eq!(original.synced_settings(), local.synced_settings());
        let saved = toml::to_string(&local).unwrap();
        let mut restored: AppSettings = toml::from_str(&saved).unwrap();
        restored.apply_synced_settings(&original.synced_settings());
        assert_eq!(restored.rdp_client, local.rdp_client);
        assert!(
            !serde_json::to_string(&local.synced_settings())
                .unwrap()
                .contains("rdp_client")
        );
        assert_eq!(
            toml::from_str::<AppSettings>("").unwrap().rdp_client,
            RdpClientSettings::default()
        );
    }

    #[test]
    fn unknown_client_kind_falls_back_without_breaking_settings() {
        let restored: AppSettings =
            toml::from_str("[rdp_client]\nkind = \"future-client\"\npath = \"/opt/future\"\n")
                .unwrap();
        assert_eq!(restored.rdp_client, RdpClientSettings::default());
        let known: AppSettings =
            toml::from_str("[rdp_client]\nkind = \"remmina\"\npath = \" /usr/bin/remmina \"\n")
                .unwrap();
        assert_eq!(
            known.rdp_client,
            RdpClientSettings::new(RdpClientKind::Remmina, "/usr/bin/remmina")
        );
    }
}
