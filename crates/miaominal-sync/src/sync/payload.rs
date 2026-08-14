use super::encryption::{decrypt_with_aad, derive_key_with_params, encrypt_with_aad};
use crate::{
    AiProviderSecret, KeySecret, LEGACY_SYNC_PAYLOAD_VERSION, PREVIOUS_SYNC_PAYLOAD_VERSION,
    PROXYLESS_SYNC_PAYLOAD_VERSION, PlaintextSecrets, ProfileSecret, ProxySecret,
    SYNC_PAYLOAD_VERSION, SyncKdf, SyncPayload, SyncPlaintextPayload, WebSearchSecret,
};
use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::profile::SessionProfile;
use miaominal_core::proxy::{ProxyAuthMode, ProxyProfile, ProxyProtocol};
use miaominal_core::snippet::SnippetRecord;
use miaominal_secrets::{SecretKind, SecretStore};
use miaominal_settings::{AppSettings, SyncedSettings};
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_storage::{ProxyStore, SettingsStore};
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

#[allow(clippy::too_many_arguments)]
pub fn build_payload(
    device_id: &str,
    parent_payload_id: Option<String>,
    plaintext: &SyncPlaintextPayload,
    passphrase: &str,
) -> Result<SyncPayload> {
    let synced_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let salt_bytes: [u8; 32] = rand::rng().random();
    let kdf = SyncKdf::argon2id(base64::engine::general_purpose::STANDARD.encode(salt_bytes));
    let mut payload = SyncPayload {
        version: SYNC_PAYLOAD_VERSION,
        device_id: device_id.to_string(),
        synced_at,
        payload_id: uuid::Uuid::new_v4().to_string(),
        parent_payload_id,
        kdf,
        encrypted_payload: String::new(),
    };
    let key = derive_key_for_kdf(passphrase, &payload.kdf)?;
    let plaintext_json =
        serde_json::to_vec(plaintext).context("failed to serialize sync plaintext")?;
    let aad = associated_data(&payload)?;
    payload.encrypted_payload = encrypt_with_aad(&key, &plaintext_json, &aad)?;

    Ok(payload)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn local_data_revision(payload: &SyncPlaintextPayload) -> Result<String> {
    let mut normalized = payload.clone();
    clear_port_forward_runtime_state(&mut normalized.sessions);
    let serialized =
        serde_json::to_vec(&normalized).context("failed to serialize local sync revision")?;
    let digest = Sha256::digest(serialized);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_plaintext_payload(
    sessions: &[SessionProfile],
    proxies: &[ProxyProfile],
    snippets: &[SnippetRecord],
    managed_keys: &[ManagedKeyRecord],
    settings: &SyncedSettings,
    secret_store: &SecretStore,
) -> Result<SyncPlaintextPayload> {
    let secrets = collect_secrets(sessions, proxies, managed_keys, settings, secret_store)?;
    let mut sessions = sessions.to_vec();
    clear_port_forward_runtime_state(&mut sessions);
    Ok(SyncPlaintextPayload {
        sessions,
        proxies: proxies.to_vec(),
        snippets: snippets.to_vec(),
        managed_keys: managed_keys.to_vec(),
        settings: settings.clone(),
        secrets,
    })
}

fn clear_port_forward_runtime_state(sessions: &mut [SessionProfile]) {
    for session in sessions {
        for rule in &mut session.port_forwarding_rules {
            rule.enabled = false;
        }
    }
}

fn merge_local_port_forward_runtime_state(
    incoming: &[SessionProfile],
    local: &[SessionProfile],
) -> Vec<SessionProfile> {
    let enabled_by_rule = local
        .iter()
        .flat_map(|profile| {
            profile
                .port_forwarding_rules
                .iter()
                .map(move |rule| ((profile.id.clone(), rule.id.clone()), rule.enabled))
        })
        .collect::<HashMap<_, _>>();
    let mut merged = incoming.to_vec();
    clear_port_forward_runtime_state(&mut merged);
    for profile in &mut merged {
        for rule in &mut profile.port_forwarding_rules {
            rule.enabled = enabled_by_rule
                .get(&(profile.id.clone(), rule.id.clone()))
                .copied()
                .unwrap_or(false);
        }
    }
    merged
}

pub fn parse_remote_payload(payload_json: &str) -> Result<SyncPayload> {
    serde_json::from_str(payload_json).context("failed to parse sync payload")
}

pub fn decrypt_remote_payload(
    payload: &SyncPayload,
    passphrase: &str,
) -> Result<SyncPlaintextPayload> {
    decrypt_payload(payload, passphrase)
}

#[allow(clippy::too_many_arguments)]
pub fn apply_plaintext_payload(
    payload: &SyncPlaintextPayload,
    session_store: &SessionStore,
    proxy_store: &ProxyStore,
    snippet_store: &SnippetStore,
    key_store: &ManagedKeyStore,
    secret_store: &SecretStore,
    settings_store: &mut SettingsStore,
    finalize: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let snapshot = PayloadSnapshot::capture(
        payload,
        session_store,
        proxy_store,
        snippet_store,
        key_store,
        secret_store,
        settings_store,
    )?;

    let apply_result = apply_payload_changes(
        payload,
        session_store,
        proxy_store,
        snippet_store,
        key_store,
        secret_store,
        settings_store,
    )
    .and_then(|()| finalize().context("failed to finalize sync pull"));

    if let Err(error) = apply_result {
        return match snapshot.restore(
            session_store,
            proxy_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
        ) {
            Ok(()) => Err(error.context("sync pull failed; local changes were rolled back")),
            Err(rollback_error) => Err(anyhow!(
                "sync pull failed: {error:#}; rollback also failed: {rollback_error:#}"
            )),
        };
    }

    Ok(())
}

fn apply_payload_changes(
    payload: &SyncPlaintextPayload,
    session_store: &SessionStore,
    proxy_store: &ProxyStore,
    snippet_store: &SnippetStore,
    key_store: &ManagedKeyStore,
    secret_store: &SecretStore,
    settings_store: &mut SettingsStore,
) -> Result<()> {
    validate_payload_proxies(payload)?;
    let old_sessions = session_store
        .read_sessions_content()?
        .map(|content| session_store.parse_sessions(&content))
        .transpose()?
        .unwrap_or_default();
    let old_proxies = proxy_store
        .read_content()?
        .map(|content| proxy_store.parse(&content))
        .transpose()?
        .unwrap_or_default();
    let old_keys = key_store.load()?;
    let old_ai_provider_ids: Vec<String> = settings_store
        .settings()
        .ai_providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect();

    for profile_secret in &payload.secrets.profile_secrets {
        if let Some(ref password) = profile_secret.password {
            secret_store.set(&profile_secret.id, SecretKind::Password, password)?;
        }
        if let Some(ref passphrase) = profile_secret.passphrase {
            secret_store.set(&profile_secret.id, SecretKind::Passphrase, passphrase)?;
        }
    }
    for key_secret in &payload.secrets.key_secrets {
        secret_store.set(
            &key_secret.id,
            SecretKind::ManagedPrivateKey,
            &key_secret.private_key,
        )?;
    }
    for provider_secret in &payload.secrets.ai_provider_secrets {
        secret_store.set(
            &provider_secret.id,
            SecretKind::AiProviderApiKey,
            &provider_secret.api_key,
        )?;
    }
    if let Some(web_search_secret) = &payload.secrets.web_search_secret {
        secret_store.set(
            "web_search",
            SecretKind::WebSearchApiKey,
            &web_search_secret.api_key,
        )?;
    }
    for proxy_secret in &payload.secrets.proxy_secrets {
        secret_store.set(
            &proxy_secret.id,
            SecretKind::ProxyPassword,
            &proxy_secret.password,
        )?;
    }

    let sessions = merge_local_port_forward_runtime_state(&payload.sessions, &old_sessions);
    proxy_store.save(&payload.proxies)?;
    session_store.save(&sessions)?;
    snippet_store.save(&payload.snippets)?;
    key_store.save(&payload.managed_keys)?;
    let mut merged_settings = settings_store.settings().clone();
    merged_settings.apply_synced_settings(&payload.settings);
    settings_store.replace(merged_settings)?;
    cleanup_removed_secrets(
        payload,
        &old_sessions,
        &old_proxies,
        &old_keys,
        &old_ai_provider_ids,
        secret_store,
    )?;

    Ok(())
}

#[derive(Debug)]
struct PayloadSnapshot {
    sessions: Vec<SessionProfile>,
    proxies: Vec<ProxyProfile>,
    snippets: Vec<SnippetRecord>,
    managed_keys: Vec<ManagedKeyRecord>,
    settings: AppSettings,
    secrets: Vec<SecretSnapshot>,
}

impl PayloadSnapshot {
    fn capture(
        payload: &SyncPlaintextPayload,
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &SettingsStore,
    ) -> Result<Self> {
        let sessions = session_store
            .read_sessions_content()?
            .map(|content| session_store.parse_sessions(&content))
            .transpose()?
            .unwrap_or_default();
        let proxies = proxy_store
            .read_content()?
            .map(|content| proxy_store.parse(&content))
            .transpose()?
            .unwrap_or_default();
        let snippets = snippet_store.load()?;
        let managed_keys = key_store.load()?;
        let settings = settings_store.settings().clone();
        let secrets = capture_affected_secrets(
            payload,
            &sessions,
            &proxies,
            &managed_keys,
            &settings,
            secret_store,
        )?;

        Ok(Self {
            sessions,
            proxies,
            snippets,
            managed_keys,
            settings,
            secrets,
        })
    }

    fn restore(
        self,
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &mut SettingsStore,
    ) -> Result<()> {
        let mut errors = Vec::new();

        if let Err(error) = session_store.save(&self.sessions) {
            errors.push(format!("sessions: {error:#}"));
        }
        if let Err(error) = proxy_store.save(&self.proxies) {
            errors.push(format!("proxies: {error:#}"));
        }
        if let Err(error) = snippet_store.save(&self.snippets) {
            errors.push(format!("snippets: {error:#}"));
        }
        if let Err(error) = key_store.save(&self.managed_keys) {
            errors.push(format!("managed keys: {error:#}"));
        }
        if let Err(error) = settings_store.replace(self.settings) {
            errors.push(format!("settings: {error:#}"));
        }
        for secret in self.secrets {
            if let Err(error) = secret.restore(secret_store) {
                errors.push(format!(
                    "secret {}/{}: {error:#}",
                    secret.id,
                    secret_kind_label(secret.kind)
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow!(errors.join("; ")))
        }
    }
}

#[derive(Debug)]
struct SecretSnapshot {
    id: String,
    kind: SecretKind,
    value: Option<String>,
}

impl SecretSnapshot {
    fn restore(&self, secret_store: &SecretStore) -> Result<()> {
        match &self.value {
            Some(value) => secret_store.set(&self.id, self.kind, value),
            None => secret_store.delete(&self.id, self.kind),
        }
    }
}

fn capture_affected_secrets(
    payload: &SyncPlaintextPayload,
    old_sessions: &[SessionProfile],
    old_proxies: &[ProxyProfile],
    old_keys: &[ManagedKeyRecord],
    old_settings: &AppSettings,
    secret_store: &SecretStore,
) -> Result<Vec<SecretSnapshot>> {
    let mut targets = Vec::new();

    for session in old_sessions.iter().chain(&payload.sessions) {
        add_secret_target(&mut targets, &session.id, SecretKind::Password);
        add_secret_target(&mut targets, &session.id, SecretKind::Passphrase);
    }
    for secret in &payload.secrets.profile_secrets {
        add_secret_target(&mut targets, &secret.id, SecretKind::Password);
        add_secret_target(&mut targets, &secret.id, SecretKind::Passphrase);
    }

    for proxy in old_proxies.iter().chain(&payload.proxies) {
        add_secret_target(&mut targets, &proxy.id, SecretKind::ProxyPassword);
    }
    for secret in &payload.secrets.proxy_secrets {
        add_secret_target(&mut targets, &secret.id, SecretKind::ProxyPassword);
    }

    for key in old_keys.iter().chain(&payload.managed_keys) {
        add_secret_target(&mut targets, &key.id, SecretKind::ManagedPrivateKey);
    }
    for secret in &payload.secrets.key_secrets {
        add_secret_target(&mut targets, &secret.id, SecretKind::ManagedPrivateKey);
    }

    for provider in old_settings
        .ai_providers
        .iter()
        .chain(&payload.settings.ai_providers)
    {
        add_secret_target(&mut targets, &provider.id, SecretKind::AiProviderApiKey);
    }
    for secret in &payload.secrets.ai_provider_secrets {
        add_secret_target(&mut targets, &secret.id, SecretKind::AiProviderApiKey);
    }
    add_secret_target(&mut targets, "web_search", SecretKind::WebSearchApiKey);

    targets
        .into_iter()
        .map(|(id, kind)| {
            let value = secret_store.get(&id, kind)?;
            Ok(SecretSnapshot { id, kind, value })
        })
        .collect()
}

fn add_secret_target(targets: &mut Vec<(String, SecretKind)>, id: &str, kind: SecretKind) {
    if targets
        .iter()
        .any(|(existing_id, existing_kind)| existing_id == id && *existing_kind == kind)
    {
        return;
    }
    targets.push((id.to_string(), kind));
}

fn secret_kind_label(kind: SecretKind) -> &'static str {
    match kind {
        SecretKind::Password => "password",
        SecretKind::Passphrase => "passphrase",
        SecretKind::ManagedPrivateKey => "managed-private-key",
        SecretKind::AiProviderApiKey => "ai-provider-api-key",
        SecretKind::WebSearchApiKey => "web-search-api-key",
        SecretKind::ProxyPassword => "proxy-password",
    }
}

fn decrypt_payload(payload: &SyncPayload, passphrase: &str) -> Result<SyncPlaintextPayload> {
    if payload.version != SYNC_PAYLOAD_VERSION
        && payload.version != PREVIOUS_SYNC_PAYLOAD_VERSION
        && payload.version != PROXYLESS_SYNC_PAYLOAD_VERSION
        && payload.version != LEGACY_SYNC_PAYLOAD_VERSION
    {
        if payload.version > SYNC_PAYLOAD_VERSION {
            anyhow::bail!(
                "sync payload version {} requires a newer Miaominal version; upgrade this device before syncing",
                payload.version
            );
        }
        anyhow::bail!("unsupported sync payload version: {}", payload.version);
    }
    let key = derive_key_for_kdf(passphrase, &payload.kdf)?;
    let aad = associated_data(payload)?;
    let plaintext_json = decrypt_with_aad(&key, &payload.encrypted_payload, &aad)?;
    deserialize_plaintext_payload(payload.version, &plaintext_json)
}

fn deserialize_plaintext_payload(
    version: u32,
    plaintext_json: &[u8],
) -> Result<SyncPlaintextPayload> {
    match version {
        SYNC_PAYLOAD_VERSION => serde_json::from_slice(plaintext_json)
            .context("failed to deserialize decrypted sync payload"),
        PREVIOUS_SYNC_PAYLOAD_VERSION => serde_json::from_slice(plaintext_json)
            .context("failed to deserialize v3 decrypted sync payload"),
        PROXYLESS_SYNC_PAYLOAD_VERSION => {
            let previous: PreviousSyncPlaintextPayload = serde_json::from_slice(plaintext_json)
                .context("failed to deserialize v2 decrypted sync payload")?;
            Ok(SyncPlaintextPayload {
                sessions: previous.sessions,
                proxies: Vec::new(),
                snippets: previous.snippets,
                managed_keys: previous.managed_keys,
                settings: previous.settings,
                secrets: previous.secrets,
            })
        }
        LEGACY_SYNC_PAYLOAD_VERSION => {
            let legacy: LegacySyncPlaintextPayload = serde_json::from_slice(plaintext_json)
                .context("failed to deserialize legacy decrypted sync payload")?;
            Ok(SyncPlaintextPayload {
                sessions: legacy.sessions,
                proxies: Vec::new(),
                snippets: legacy.snippets,
                managed_keys: legacy.managed_keys,
                settings: legacy.settings.synced_settings(),
                secrets: legacy.secrets,
            })
        }
        _ => anyhow::bail!("unsupported sync payload version: {version}"),
    }
}

const MAX_KDF_MEMORY_COST_KIB: u32 = 512 * 1024; // 512 MiB
const MAX_KDF_TIME_COST: u32 = 20;
const MAX_KDF_PARALLELISM: u32 = 16;

fn derive_key_for_kdf(passphrase: &str, kdf: &SyncKdf) -> Result<[u8; 32]> {
    if kdf.algorithm != "argon2id" {
        anyhow::bail!("unsupported sync KDF algorithm: {}", kdf.algorithm);
    }
    if kdf.version != 0x13 {
        anyhow::bail!("unsupported Argon2 version: {}", kdf.version);
    }
    if kdf.memory_cost > MAX_KDF_MEMORY_COST_KIB {
        anyhow::bail!(
            "sync KDF memory cost {} KiB exceeds limit of {} KiB",
            kdf.memory_cost,
            MAX_KDF_MEMORY_COST_KIB
        );
    }
    if kdf.time_cost > MAX_KDF_TIME_COST {
        anyhow::bail!(
            "sync KDF time cost {} exceeds limit of {}",
            kdf.time_cost,
            MAX_KDF_TIME_COST
        );
    }
    if kdf.parallelism > MAX_KDF_PARALLELISM {
        anyhow::bail!(
            "sync KDF parallelism {} exceeds limit of {}",
            kdf.parallelism,
            MAX_KDF_PARALLELISM
        );
    }
    let salt = base64::engine::general_purpose::STANDARD
        .decode(&kdf.salt)
        .context("failed to decode sync KDF salt")?;
    if salt.len() != 32 {
        anyhow::bail!("sync KDF salt must be 32 bytes");
    }
    derive_key_with_params(
        passphrase,
        &salt,
        kdf.memory_cost,
        kdf.time_cost,
        kdf.parallelism,
        kdf.output_len,
    )
}

#[derive(Serialize)]
struct SyncPayloadAssociatedData<'a> {
    version: u32,
    device_id: &'a str,
    synced_at: u64,
    #[serde(skip_serializing_if = "str::is_empty")]
    payload_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_payload_id: Option<&'a str>,
    kdf: &'a SyncKdf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacySyncPlaintextPayload {
    sessions: Vec<SessionProfile>,
    snippets: Vec<SnippetRecord>,
    managed_keys: Vec<ManagedKeyRecord>,
    settings: AppSettings,
    #[serde(default)]
    secrets: PlaintextSecrets,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PreviousSyncPlaintextPayload {
    sessions: Vec<SessionProfile>,
    snippets: Vec<SnippetRecord>,
    managed_keys: Vec<ManagedKeyRecord>,
    settings: SyncedSettings,
    #[serde(default)]
    secrets: PlaintextSecrets,
}

fn associated_data(payload: &SyncPayload) -> Result<Vec<u8>> {
    serde_json::to_vec(&SyncPayloadAssociatedData {
        version: payload.version,
        device_id: &payload.device_id,
        synced_at: payload.synced_at,
        payload_id: &payload.payload_id,
        parent_payload_id: payload.parent_payload_id.as_deref(),
        kdf: &payload.kdf,
    })
    .context("failed to serialize sync associated data")
}

fn collect_secrets(
    sessions: &[SessionProfile],
    proxies: &[ProxyProfile],
    managed_keys: &[ManagedKeyRecord],
    settings: &SyncedSettings,
    secret_store: &SecretStore,
) -> Result<PlaintextSecrets> {
    let mut profile_secrets = Vec::new();
    for session in sessions {
        let password = secret_store.get(&session.id, SecretKind::Password)?;
        let passphrase = secret_store.get(&session.id, SecretKind::Passphrase)?;
        if password.is_some() || passphrase.is_some() {
            profile_secrets.push(ProfileSecret {
                id: session.id.clone(),
                password,
                passphrase,
            });
        }
    }

    let mut key_secrets = Vec::new();
    for key in managed_keys {
        if let Some(private_key) = secret_store.get(&key.id, SecretKind::ManagedPrivateKey)? {
            key_secrets.push(KeySecret {
                id: key.id.clone(),
                private_key,
            });
        }
    }

    let mut ai_provider_secrets = Vec::new();
    for provider in &settings.ai_providers {
        if provider.has_api_key
            && let Some(api_key) = secret_store.get(&provider.id, SecretKind::AiProviderApiKey)?
        {
            ai_provider_secrets.push(AiProviderSecret {
                id: provider.id.clone(),
                api_key,
            });
        }
    }

    let web_search_secret = if settings.web_search.has_api_key {
        secret_store
            .get("web_search", SecretKind::WebSearchApiKey)?
            .map(|api_key| WebSearchSecret { api_key })
    } else {
        None
    };

    let mut proxy_secrets = Vec::new();
    for proxy in proxies {
        if proxy.auth_mode != ProxyAuthMode::UsernamePassword || !proxy.has_stored_password {
            continue;
        }
        if let Some(password) = secret_store.get(&proxy.id, SecretKind::ProxyPassword)? {
            proxy_secrets.push(ProxySecret {
                id: proxy.id.clone(),
                password,
            });
        }
    }

    Ok(PlaintextSecrets {
        profile_secrets,
        key_secrets,
        ai_provider_secrets,
        web_search_secret,
        proxy_secrets,
    })
}

fn cleanup_removed_secrets(
    payload: &SyncPlaintextPayload,
    old_sessions: &[SessionProfile],
    old_proxies: &[ProxyProfile],
    old_keys: &[ManagedKeyRecord],
    old_ai_provider_ids: &[String],
    secret_store: &SecretStore,
) -> Result<()> {
    let profile_ids: HashSet<&str> = payload
        .sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    let profile_password_ids: HashSet<&str> = payload
        .secrets
        .profile_secrets
        .iter()
        .filter(|secret| secret.password.is_some())
        .map(|secret| secret.id.as_str())
        .collect();
    let profile_passphrase_ids: HashSet<&str> = payload
        .secrets
        .profile_secrets
        .iter()
        .filter(|secret| secret.passphrase.is_some())
        .map(|secret| secret.id.as_str())
        .collect();
    for session in &payload.sessions {
        if !profile_password_ids.contains(session.id.as_str()) {
            secret_store.delete(&session.id, SecretKind::Password)?;
        }
        if !profile_passphrase_ids.contains(session.id.as_str()) {
            secret_store.delete(&session.id, SecretKind::Passphrase)?;
        }
    }
    for session in old_sessions {
        if !profile_ids.contains(session.id.as_str()) {
            secret_store.delete(&session.id, SecretKind::Password)?;
            secret_store.delete(&session.id, SecretKind::Passphrase)?;
        }
    }

    let proxy_ids: HashSet<&str> = payload
        .proxies
        .iter()
        .map(|proxy| proxy.id.as_str())
        .collect();
    let proxy_secret_ids: HashSet<&str> = payload
        .secrets
        .proxy_secrets
        .iter()
        .map(|secret| secret.id.as_str())
        .collect();
    for proxy in &payload.proxies {
        if !proxy_secret_ids.contains(proxy.id.as_str()) {
            secret_store.delete(&proxy.id, SecretKind::ProxyPassword)?;
        }
    }
    for proxy in old_proxies {
        if !proxy_ids.contains(proxy.id.as_str()) {
            secret_store.delete(&proxy.id, SecretKind::ProxyPassword)?;
        }
    }

    let key_ids: HashSet<&str> = payload
        .managed_keys
        .iter()
        .map(|key| key.id.as_str())
        .collect();
    for key in old_keys {
        if !key_ids.contains(key.id.as_str()) {
            secret_store.delete(&key.id, SecretKind::ManagedPrivateKey)?;
        }
    }

    let provider_ids: HashSet<&str> = payload
        .settings
        .ai_providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect();
    for provider in &payload.settings.ai_providers {
        if !provider.has_api_key {
            secret_store.delete(&provider.id, SecretKind::AiProviderApiKey)?;
        }
    }
    for old_provider_id in old_ai_provider_ids {
        if !provider_ids.contains(old_provider_id.as_str()) {
            secret_store.delete(old_provider_id, SecretKind::AiProviderApiKey)?;
        }
    }

    if !payload.settings.web_search.has_api_key {
        secret_store.delete("web_search", SecretKind::WebSearchApiKey)?;
    }

    Ok(())
}

fn validate_payload_proxies(payload: &SyncPlaintextPayload) -> Result<()> {
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    let mut secret_ids = HashSet::new();
    for secret in &payload.secrets.proxy_secrets {
        if secret.password.is_empty() || !secret_ids.insert(secret.id.as_str()) {
            anyhow::bail!("sync payload contains an invalid or duplicate proxy secret");
        }
    }

    for proxy in &payload.proxies {
        if proxy.id.trim().is_empty() || !ids.insert(proxy.id.as_str()) {
            anyhow::bail!("sync payload contains an empty or duplicate proxy id");
        }
        let normalized_name = proxy.name.trim().to_ascii_lowercase();
        if normalized_name.is_empty() || !names.insert(normalized_name) {
            anyhow::bail!("sync payload contains an empty or duplicate proxy name");
        }
        if proxy.host.trim().is_empty()
            || proxy.host.chars().any(char::is_control)
            || proxy.host.chars().any(char::is_whitespace)
            || proxy.port == 0
        {
            anyhow::bail!("sync payload contains an invalid proxy endpoint");
        }
        match proxy.auth_mode {
            ProxyAuthMode::None => {
                if secret_ids.contains(proxy.id.as_str()) {
                    anyhow::bail!("sync payload contains a password for an unauthenticated proxy");
                }
            }
            ProxyAuthMode::UsernamePassword => {
                if proxy.username.trim().is_empty() {
                    anyhow::bail!("sync payload proxy authentication username is missing");
                }
                if proxy.protocol == ProxyProtocol::HttpConnect && proxy.username.contains(':') {
                    anyhow::bail!("sync payload HTTP proxy username contains a colon");
                }
            }
        }
    }
    if secret_ids.iter().any(|id| !ids.contains(id)) {
        anyhow::bail!("sync payload contains a secret for a missing proxy");
    }
    for session in &payload.sessions {
        if let Some(proxy_id) = session.entry_proxy_id.as_deref()
            && !ids.contains(proxy_id)
        {
            anyhow::bail!(
                "sync payload host {} references missing proxy {}",
                session.connection_label(),
                proxy_id
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::keychain::ManagedKeySource;
    use miaominal_core::profile::PortForwardRule;
    use miaominal_secrets::{
        APP_CREDENTIAL_SERVICE, CredentialStore, ProtectedPassphrase, VaultCredentialBackend,
        set_vault_test_parameters,
    };

    #[test]
    fn payload_decrypts_with_remote_salt() {
        let plaintext = sample_plaintext();
        let payload = encrypted_payload("correct horse", &plaintext);

        let decrypted = decrypt_payload(&payload, "correct horse").expect("payload should decrypt");

        assert_eq!(decrypted.sessions.len(), 1);
        assert_eq!(decrypted.sessions[0].id, "session-1");
        assert_eq!(decrypted.secrets.profile_secrets.len(), 1);
        assert_eq!(
            decrypted.secrets.profile_secrets[0].password.as_deref(),
            Some("password")
        );
    }

    #[test]
    fn payload_rejects_wrong_passphrase() {
        let plaintext = sample_plaintext();
        let payload = encrypted_payload("correct horse", &plaintext);

        assert!(decrypt_payload(&payload, "wrong horse").is_err());
    }

    #[test]
    fn payload_rejects_metadata_tampering() {
        let plaintext = sample_plaintext();
        let mut payload = encrypted_payload("correct horse", &plaintext);
        payload.synced_at += 1;

        assert!(decrypt_payload(&payload, "correct horse").is_err());
    }

    #[test]
    fn payload_rejects_parent_identity_tampering() {
        let plaintext = sample_plaintext();
        let mut payload = encrypted_payload("correct horse", &plaintext);
        payload.parent_payload_id = Some("different-parent".into());

        assert!(decrypt_payload(&payload, "correct horse").is_err());
    }

    #[test]
    fn local_revision_is_stable_and_tracks_payload_content() {
        let first = sample_plaintext();
        let mut changed = first.clone();
        changed.settings.recent_connections_count += 1;

        assert_eq!(
            local_data_revision(&first).unwrap(),
            local_data_revision(&first).unwrap()
        );
        assert_ne!(
            local_data_revision(&first).unwrap(),
            local_data_revision(&changed).unwrap()
        );
    }

    #[test]
    fn port_forward_enabled_is_not_part_of_local_revision() {
        let mut disabled = sample_plaintext();
        let rule = test_port_forward_rule("forward-1", false);
        disabled.sessions[0].port_forwarding_rules.push(rule);
        let mut enabled = disabled.clone();
        enabled.sessions[0].port_forwarding_rules[0].enabled = true;

        assert_eq!(
            local_data_revision(&disabled).unwrap(),
            local_data_revision(&enabled).unwrap()
        );
    }

    #[test]
    fn sync_sessions_clear_runtime_enabled_without_mutating_input() {
        let mut input = sample_plaintext().sessions;
        let rule = test_port_forward_rule("forward-1", true);
        input[0].port_forwarding_rules.push(rule);

        let mut normalized = input.clone();
        clear_port_forward_runtime_state(&mut normalized);

        assert!(input[0].port_forwarding_rules[0].enabled);
        assert!(!normalized[0].port_forwarding_rules[0].enabled);
    }

    #[test]
    fn pull_preserves_only_matching_local_port_forward_runtime_state() {
        let mut local = sample_plaintext().sessions;
        let local_rule = test_port_forward_rule("existing", true);
        local[0].port_forwarding_rules.push(local_rule);

        let mut incoming = local.clone();
        incoming[0].port_forwarding_rules[0].enabled = false;
        let remote_only = test_port_forward_rule("remote-only", true);
        incoming[0].port_forwarding_rules.push(remote_only);

        let merged = merge_local_port_forward_runtime_state(&incoming, &local);

        assert!(merged[0].port_forwarding_rules[0].enabled);
        assert!(!merged[0].port_forwarding_rules[1].enabled);
    }

    #[test]
    fn payload_rejects_unknown_version_before_decryption() {
        let mut payload = encrypted_payload("correct horse", &sample_plaintext());
        payload.version = SYNC_PAYLOAD_VERSION + 1;

        let error = decrypt_payload(&payload, "correct horse")
            .expect_err("unknown sync versions should be rejected");
        assert!(error.to_string().contains("upgrade this device"));
    }

    #[test]
    fn payload_reads_legacy_v1_plaintext() {
        let settings = AppSettings {
            theme_id: miaominal_settings::ThemeId::Dark,
            font_family: "JetBrains Mono".into(),
            recent_connections_count: 9,
            auto_collect_session_monitoring: true,
            ..AppSettings::default()
        };

        let plaintext = LegacySyncPlaintextPayload {
            sessions: sample_plaintext().sessions,
            snippets: Vec::new(),
            managed_keys: Vec::new(),
            settings: settings.clone(),
            secrets: PlaintextSecrets {
                profile_secrets: vec![ProfileSecret {
                    id: "session-1".into(),
                    password: Some("password".into()),
                    passphrase: None,
                }],
                key_secrets: Vec::new(),
                ai_provider_secrets: Vec::new(),
                web_search_secret: None,
                proxy_secrets: Vec::new(),
            },
        };

        let decrypted = decrypt_payload(
            &legacy_encrypted_payload("correct horse", &plaintext),
            "correct horse",
        )
        .expect("legacy payload should decrypt");

        assert_eq!(decrypted.sessions.len(), 1);
        assert!(decrypted.proxies.is_empty());
        assert_eq!(decrypted.settings, settings.synced_settings());
        assert_eq!(
            decrypted.secrets.profile_secrets[0].password.as_deref(),
            Some("password")
        );
    }

    #[test]
    fn payload_reads_v2_plaintext_with_empty_proxies() {
        let sample = sample_plaintext();
        let previous = PreviousSyncPlaintextPayload {
            sessions: sample.sessions,
            snippets: sample.snippets,
            managed_keys: sample.managed_keys,
            settings: sample.settings,
            secrets: sample.secrets,
        };
        let decrypted = decrypt_payload(
            &encrypted_payload_with_version(
                "correct horse",
                PROXYLESS_SYNC_PAYLOAD_VERSION,
                &previous,
            ),
            "correct horse",
        )
        .expect("v2 payload should decrypt");

        assert!(decrypted.proxies.is_empty());
        assert_eq!(decrypted.sessions.len(), 1);
    }

    #[test]
    fn v3_payload_remains_decryptable() {
        let plaintext = sample_proxy_plaintext();
        let decrypted = decrypt_payload(
            &encrypted_payload_with_version(
                "correct horse",
                PREVIOUS_SYNC_PAYLOAD_VERSION,
                &plaintext,
            ),
            "correct horse",
        )
        .expect("v3 proxy payload should decrypt");

        assert_eq!(decrypted.proxies.len(), 1);
        assert_eq!(decrypted.secrets.proxy_secrets.len(), 1);
    }

    #[test]
    fn v4_payload_round_trips_proxy_metadata_and_password() {
        let plaintext = sample_proxy_plaintext();
        let decrypted = decrypt_payload(
            &encrypted_payload("correct horse", &plaintext),
            "correct horse",
        )
        .expect("v3 proxy payload should decrypt");

        assert_eq!(decrypted.proxies.len(), 1);
        assert_eq!(decrypted.proxies[0].id, "proxy-1");
        assert_eq!(
            decrypted.sessions[0].entry_proxy_id.as_deref(),
            Some("proxy-1")
        );
        assert_eq!(decrypted.secrets.proxy_secrets.len(), 1);
        assert_eq!(
            decrypted.secrets.proxy_secrets[0].password,
            "proxy-password"
        );
    }

    #[test]
    fn v3_payload_rejects_dangling_proxy_reference() {
        let mut plaintext = sample_plaintext();
        plaintext.sessions[0].entry_proxy_id = Some("missing-proxy".into());

        let error = validate_payload_proxies(&plaintext)
            .expect_err("dangling proxy references should reject the full payload");
        assert!(error.to_string().contains("missing proxy"));
    }

    #[test]
    fn v3_payload_allows_an_explicitly_cleared_proxy_password() {
        let mut plaintext = sample_proxy_plaintext();
        plaintext.secrets.proxy_secrets.clear();
        plaintext.proxies[0].has_stored_password = false;

        validate_payload_proxies(&plaintext)
            .expect("cleared proxy password should remain a valid synced configuration");
    }

    #[test]
    fn pull_clears_stale_password_for_proxy_that_remains_in_payload() {
        set_vault_test_parameters();
        let vault_path = std::env::temp_dir().join(format!(
            "miaominal-payload-cleared-proxy-secret-{}.json",
            uuid::Uuid::new_v4()
        ));
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                vault_path.clone(),
                ProtectedPassphrase::try_from_string("proxy-secret-test".to_string())
                    .expect("test passphrase should use protected memory"),
            ),
        );
        credentials
            .initialize()
            .expect("test credential store should initialize");
        let secret_store = SecretStore::with_credentials(credentials);
        secret_store
            .set("proxy-1", SecretKind::ProxyPassword, "stale-password")
            .expect("stale proxy password should save");
        let mut payload = sample_proxy_plaintext();
        payload.secrets.proxy_secrets.clear();
        payload.proxies[0].has_stored_password = false;

        cleanup_removed_secrets(&payload, &[], &payload.proxies, &[], &[], &secret_store)
            .expect("cleared proxy password should be removed locally");

        assert_eq!(
            secret_store
                .get("proxy-1", SecretKind::ProxyPassword)
                .expect("proxy password state should read"),
            None
        );
        assert!(
            collect_secrets(
                &payload.sessions,
                &payload.proxies,
                &payload.managed_keys,
                &payload.settings,
                &secret_store,
            )
            .expect("secrets should collect after clearing")
            .proxy_secrets
            .is_empty(),
            "the cleared password must not be resurrected by the next push"
        );
        cleanup_test_vault(&vault_path);
    }

    #[test]
    fn collect_secrets_ignores_stale_proxy_passwords_disallowed_by_metadata() {
        set_vault_test_parameters();
        let vault_path = std::env::temp_dir().join(format!(
            "miaominal-payload-unauthenticated-proxy-secret-{}.json",
            uuid::Uuid::new_v4()
        ));
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                vault_path.clone(),
                ProtectedPassphrase::try_from_string("proxy-secret-test".to_string())
                    .expect("test passphrase should use protected memory"),
            ),
        );
        credentials
            .initialize()
            .expect("test credential store should initialize");
        let secret_store = SecretStore::with_credentials(credentials);
        secret_store
            .set("proxy-1", SecretKind::ProxyPassword, "stale-password")
            .expect("stale proxy password should save");
        let mut payload = sample_proxy_plaintext();
        payload.proxies[0].auth_mode = ProxyAuthMode::None;
        payload.proxies[0].username.clear();
        payload.proxies[0].has_stored_password = true;

        payload.secrets = collect_secrets(
            &payload.sessions,
            &payload.proxies,
            &payload.managed_keys,
            &payload.settings,
            &secret_store,
        )
        .expect("secrets should collect");

        assert!(payload.secrets.proxy_secrets.is_empty());
        validate_payload_proxies(&payload)
            .expect("an unauthenticated proxy must not produce a poisoned payload");

        payload.proxies[0].auth_mode = ProxyAuthMode::UsernamePassword;
        payload.proxies[0].username = "alice".into();
        payload.proxies[0].has_stored_password = false;
        payload.secrets = collect_secrets(
            &payload.sessions,
            &payload.proxies,
            &payload.managed_keys,
            &payload.settings,
            &secret_store,
        )
        .expect("secrets should collect after an explicit password clear");

        assert!(payload.secrets.proxy_secrets.is_empty());
        validate_payload_proxies(&payload)
            .expect("an explicitly cleared proxy password must stay cleared");
        cleanup_test_vault(&vault_path);
    }

    #[test]
    fn pull_reconciles_cleared_and_missing_profile_secrets() {
        set_vault_test_parameters();
        let root = std::env::temp_dir().join(format!(
            "miaominal-payload-cleared-profile-secrets-{}",
            uuid::Uuid::new_v4()
        ));
        let session_store = SessionStore::with_path(root.join("sessions.toml"));
        let proxy_store = ProxyStore::with_path(root.join("proxies.toml"));
        let snippet_store = SnippetStore::with_path(root.join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(root.join("managed_keys.toml"));
        let mut settings_store = SettingsStore::load_with_path(root.join("settings.toml"))
            .expect("settings store should load");
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                root.join("secret_vault.json"),
                ProtectedPassphrase::try_from_string("profile-secret-test".to_string())
                    .expect("test passphrase should use protected memory"),
            ),
        );
        credentials
            .initialize()
            .expect("test credential store should initialize");
        let secret_store = SecretStore::with_credentials(credentials);

        let mut payload = sample_plaintext();
        payload.secrets.profile_secrets[0].password = None;
        payload.secrets.profile_secrets[0].passphrase = Some("new-passphrase".into());
        let mut session_without_secrets = SessionProfile::blank("session-2", 2);
        session_without_secrets.host = "second.example.com".into();
        payload.sessions.push(session_without_secrets);
        session_store
            .save(&payload.sessions)
            .expect("existing sessions should save");

        for session_id in ["session-1", "session-2"] {
            secret_store
                .set(session_id, SecretKind::Password, "stale-password")
                .expect("stale password should save");
            secret_store
                .set(session_id, SecretKind::Passphrase, "stale-passphrase")
                .expect("stale passphrase should save");
        }

        apply_plaintext_payload(
            &payload,
            &session_store,
            &proxy_store,
            &snippet_store,
            &key_store,
            &secret_store,
            &mut settings_store,
            || Ok(()),
        )
        .expect("profile secret reconciliation should succeed");

        assert_eq!(
            secret_store
                .get("session-1", SecretKind::Password)
                .expect("cleared password state should read"),
            None
        );
        assert_eq!(
            secret_store
                .get("session-1", SecretKind::Passphrase)
                .expect("updated passphrase should read")
                .as_deref(),
            Some("new-passphrase")
        );
        for kind in [SecretKind::Password, SecretKind::Passphrase] {
            assert_eq!(
                secret_store
                    .get("session-2", kind)
                    .expect("missing profile secret state should read"),
                None
            );
        }

        let collected = collect_secrets(
            &payload.sessions,
            &payload.proxies,
            &payload.managed_keys,
            &payload.settings,
            &secret_store,
        )
        .expect("secrets should collect after reconciliation");
        assert_eq!(collected.profile_secrets.len(), 1);
        assert_eq!(collected.profile_secrets[0].id, "session-1");
        assert_eq!(collected.profile_secrets[0].password, None);
        assert_eq!(
            collected.profile_secrets[0].passphrase.as_deref(),
            Some("new-passphrase")
        );

        drop(secret_store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn collect_secrets_includes_ai_provider_api_keys() {
        set_vault_test_parameters();

        let provider = miaominal_settings::AiProviderConfig {
            id: "provider-1".into(),
            name: "OpenAI".into(),
            kind: miaominal_settings::AiProviderKind::OpenAi,
            model: "gpt-4o".into(),
            base_url: String::new(),
            api_key_env: String::new(),
            has_api_key: true,
            enabled: true,
            context_window: None,
            temperature: Some(0.7),
            max_tokens: Some(1280000),
            reasoning_effort: miaominal_settings::AiReasoningEffort::Default,
        };
        let settings = AppSettings {
            ai_providers: vec![provider],
            ..AppSettings::default()
        }
        .synced_settings();
        let vault_path = std::env::temp_dir().join(format!(
            "miaominal-payload-provider-secret-{}.json",
            uuid::Uuid::new_v4()
        ));
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                vault_path.clone(),
                ProtectedPassphrase::try_from_string("provider-secret-test".to_string())
                    .expect("test passphrase should use protected memory"),
            ),
        );
        credentials
            .initialize()
            .expect("test credential store should initialize");
        let secret_store = SecretStore::with_credentials(credentials);
        secret_store
            .set("provider-1", SecretKind::AiProviderApiKey, "sk-test")
            .expect("provider api key should save");

        let secrets = collect_secrets(&[], &[], &[], &settings, &secret_store)
            .expect("secrets should collect");

        assert_eq!(secrets.ai_provider_secrets.len(), 1);
        assert_eq!(secrets.ai_provider_secrets[0].id, "provider-1");
        assert_eq!(secrets.ai_provider_secrets[0].api_key, "sk-test");
        cleanup_test_vault(&vault_path);
    }

    #[test]
    fn apply_payload_rolls_back_every_store_when_final_commit_fails() {
        set_vault_test_parameters();
        let root = std::env::temp_dir().join(format!(
            "miaominal-payload-transaction-{}",
            uuid::Uuid::new_v4()
        ));
        let session_store = SessionStore::with_path(root.join("sessions.toml"));
        let proxy_store = ProxyStore::with_path(root.join("proxies.toml"));
        let snippet_store = SnippetStore::with_path(root.join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(root.join("managed_keys.toml"));
        let mut settings_store = SettingsStore::load_with_path(root.join("settings.toml"))
            .expect("settings store should load");
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                root.join("secret_vault.json"),
                ProtectedPassphrase::try_from_string("vault-passphrase".to_string())
                    .expect("test passphrase should use protected memory"),
            ),
        );
        let secret_store = SecretStore::with_credentials(credentials);

        let mut old_session = SessionProfile::blank("old-session", 1);
        old_session.host = "old.example.com".into();
        let old_snippet = SnippetRecord {
            id: "old-snippet".into(),
            description: "Old snippet".into(),
            package: "ops".into(),
            language: "bash".into(),
            script: "echo old".into(),
        };
        let old_key = ManagedKeyRecord {
            id: "old-key".into(),
            name: "Old key".into(),
            algorithm: "ssh-ed25519".into(),
            public_key: "ssh-ed25519 AAAA".into(),
            source: ManagedKeySource::Generated,
        };
        session_store
            .save(std::slice::from_ref(&old_session))
            .expect("old sessions should save");
        snippet_store
            .save(std::slice::from_ref(&old_snippet))
            .expect("old snippets should save");
        key_store
            .save(std::slice::from_ref(&old_key))
            .expect("old keys should save");
        secret_store
            .set("old-session", SecretKind::Password, "old-password")
            .expect("old password should save");
        secret_store
            .set("old-key", SecretKind::ManagedPrivateKey, "old-private-key")
            .expect("old private key should save");
        let mut old_settings = settings_store.settings().clone();
        old_settings.font_family = "Old Font".into();
        settings_store
            .replace(old_settings.clone())
            .expect("old settings should save");

        let result = apply_plaintext_payload(
            &sample_plaintext(),
            &session_store,
            &proxy_store,
            &snippet_store,
            &key_store,
            &secret_store,
            &mut settings_store,
            || Err(anyhow!("simulated sync config failure")),
        );

        let error = result.expect_err("final commit should fail");
        assert!(error.to_string().contains("rolled back"));
        let restored_sessions = session_store
            .read_sessions_content()
            .expect("sessions should read")
            .map(|content| session_store.parse_sessions(&content))
            .transpose()
            .expect("sessions should parse")
            .unwrap_or_default();
        assert_eq!(restored_sessions.len(), 1);
        assert_eq!(restored_sessions[0].id, old_session.id);
        assert_eq!(
            snippet_store.load().expect("snippets should load"),
            vec![old_snippet]
        );
        assert_eq!(key_store.load().expect("keys should load"), vec![old_key]);
        assert_eq!(
            settings_store.settings().font_family,
            old_settings.font_family
        );
        assert_eq!(
            secret_store
                .get("old-session", SecretKind::Password)
                .expect("old password should read")
                .as_deref(),
            Some("old-password")
        );
        assert_eq!(
            secret_store
                .get("old-key", SecretKind::ManagedPrivateKey)
                .expect("old private key should read")
                .as_deref(),
            Some("old-private-key")
        );
        assert_eq!(
            secret_store
                .get("session-1", SecretKind::Password)
                .expect("new password state should read"),
            None
        );

        let _ = std::fs::remove_dir_all(root);
    }

    fn encrypted_payload(passphrase: &str, plaintext: &SyncPlaintextPayload) -> SyncPayload {
        encrypted_payload_with_version(passphrase, SYNC_PAYLOAD_VERSION, plaintext)
    }

    fn legacy_encrypted_payload(
        passphrase: &str,
        plaintext: &LegacySyncPlaintextPayload,
    ) -> SyncPayload {
        encrypted_payload_with_version(passphrase, LEGACY_SYNC_PAYLOAD_VERSION, plaintext)
    }

    fn encrypted_payload_with_version<T: Serialize>(
        passphrase: &str,
        version: u32,
        plaintext: &T,
    ) -> SyncPayload {
        let salt = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        let mut payload = SyncPayload {
            version,
            device_id: "device-1".into(),
            synced_at: 42,
            payload_id: if version == SYNC_PAYLOAD_VERSION {
                "payload-1".into()
            } else {
                String::new()
            },
            parent_payload_id: None,
            kdf: SyncKdf::argon2id(salt),
            encrypted_payload: String::new(),
        };
        let key = derive_key_for_kdf(passphrase, &payload.kdf).expect("key should derive");
        let aad = associated_data(&payload).expect("aad should serialize");
        payload.encrypted_payload = encrypt_with_aad(
            &key,
            &serde_json::to_vec(plaintext).expect("plaintext should serialize"),
            &aad,
        )
        .expect("payload should encrypt");
        payload
    }

    fn sample_plaintext() -> SyncPlaintextPayload {
        let mut session = SessionProfile::blank("session-1", 1);
        session.host = "example.com".into();
        session.username = "akko".into();

        SyncPlaintextPayload {
            sessions: vec![session],
            proxies: Vec::new(),
            snippets: Vec::new(),
            managed_keys: Vec::new(),
            settings: AppSettings::default().synced_settings(),
            secrets: PlaintextSecrets {
                profile_secrets: vec![ProfileSecret {
                    id: "session-1".into(),
                    password: Some("password".into()),
                    passphrase: None,
                }],
                key_secrets: Vec::new(),
                ai_provider_secrets: Vec::new(),
                web_search_secret: None,
                proxy_secrets: Vec::new(),
            },
        }
    }

    fn test_port_forward_rule(id: &str, enabled: bool) -> PortForwardRule {
        PortForwardRule {
            id: id.into(),
            label: String::new(),
            kind: Default::default(),
            listen_host: "127.0.0.1".into(),
            listen_port: 1000,
            target_host: "127.0.0.1".into(),
            target_port: 2000,
            enabled,
        }
    }

    fn sample_proxy_plaintext() -> SyncPlaintextPayload {
        let mut payload = sample_plaintext();
        let mut proxy = ProxyProfile::blank("proxy-1", 1);
        proxy.name = "Shared proxy".into();
        proxy.host = "127.0.0.1".into();
        proxy.auth_mode = ProxyAuthMode::UsernamePassword;
        proxy.username = "akko".into();
        proxy.has_stored_password = true;
        payload.sessions[0].entry_proxy_id = Some(proxy.id.clone());
        payload.proxies.push(proxy);
        payload.secrets.proxy_secrets.push(ProxySecret {
            id: "proxy-1".into(),
            password: "proxy-password".into(),
        });
        payload
    }

    fn cleanup_test_vault(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let _ = std::fs::remove_file(std::path::PathBuf::from(lock_path));
    }
}
