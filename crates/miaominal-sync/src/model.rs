use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::profile::SessionProfile;
use miaominal_core::proxy::ProxyProfile;
use miaominal_core::snippet::SnippetRecord;
use miaominal_settings::SyncedSettings;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SyncProvider {
    #[default]
    None,
    GithubGist,
    WebDav,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncConfig {
    /// Monotonic generation for optimistic coordination between independent
    /// SyncConfigStore clones in this process.
    #[serde(default)]
    pub config_revision: u64,
    #[serde(default)]
    pub provider: SyncProvider,
    #[serde(default)]
    pub gist_enabled: bool,
    #[serde(default)]
    pub webdav_enabled: bool,
    #[serde(default)]
    pub gist_id: Option<String>,
    #[serde(default)]
    pub webdav_url: String,
    #[serde(default)]
    pub webdav_username: String,
    #[serde(default)]
    pub has_github_token: bool,
    #[serde(default)]
    pub has_webdav_password: bool,
    #[serde(default)]
    pub has_passphrase: bool,
    #[serde(default)]
    pub last_sync_at: u64,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub auto_sync_enabled: bool,
    #[serde(default)]
    pub remote_etag: Option<String>,
    #[serde(default)]
    pub remote_payload_id: Option<String>,
    #[serde(default)]
    pub last_synced_local_revision: Option<String>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            config_revision: 0,
            provider: SyncProvider::None,
            gist_enabled: true,
            webdav_enabled: true,
            gist_id: None,
            webdav_url: String::new(),
            webdav_username: String::new(),
            has_github_token: false,
            has_webdav_password: false,
            has_passphrase: false,
            last_sync_at: 0,
            device_id: String::new(),
            auto_sync_enabled: false,
            remote_etag: None,
            remote_payload_id: None,
            last_synced_local_revision: None,
        }
    }
}

impl SyncConfig {
    pub fn normalize_legacy_provider_flags(&mut self) {
        self.gist_enabled = true;
        self.webdav_enabled = true;
    }
}

pub const LEGACY_SYNC_PAYLOAD_VERSION: u32 = 1;
pub const PROXYLESS_SYNC_PAYLOAD_VERSION: u32 = 2;
pub const PREVIOUS_SYNC_PAYLOAD_VERSION: u32 = 3;
pub const SYNC_PAYLOAD_VERSION: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncKdf {
    pub algorithm: String,
    pub version: u32,
    pub memory_cost: u32,
    pub time_cost: u32,
    pub parallelism: u32,
    pub output_len: usize,
    pub salt: String,
}

impl SyncKdf {
    pub fn argon2id(salt: String) -> Self {
        Self {
            algorithm: "argon2id".to_string(),
            version: 0x13,
            memory_cost: 65536,
            time_cost: 3,
            parallelism: 4,
            output_len: 32,
            salt,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPayload {
    pub version: u32,
    pub device_id: String,
    pub synced_at: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub payload_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_payload_id: Option<String>,
    pub kdf: SyncKdf,
    pub encrypted_payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPlaintextPayload {
    pub sessions: Vec<SessionProfile>,
    pub proxies: Vec<ProxyProfile>,
    pub snippets: Vec<SnippetRecord>,
    pub managed_keys: Vec<ManagedKeyRecord>,
    pub settings: SyncedSettings,
    #[serde(default)]
    pub secrets: PlaintextSecrets,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaintextSecrets {
    #[serde(default)]
    pub profile_secrets: Vec<ProfileSecret>,
    #[serde(default)]
    pub key_secrets: Vec<KeySecret>,
    #[serde(default)]
    pub ai_provider_secrets: Vec<AiProviderSecret>,
    #[serde(default)]
    pub web_search_secret: Option<WebSearchSecret>,
    #[serde(default)]
    pub proxy_secrets: Vec<ProxySecret>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSecret {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeySecret {
    pub id: String,
    pub private_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderSecret {
    pub id: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchSecret {
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySecret {
    pub id: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    Idle,
    Syncing,
    RemoteBindingRequired { provider: SyncProvider },
    Pulled { at: u64 },
    Pushed { at: u64 },
    PullRequired { remote_at: u64 },
    UpToDate { at: u64 },
    Error(String),
}
