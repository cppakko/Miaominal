use super::encryption::{decrypt_with_aad, derive_key_with_params, encrypt_with_aad};
use crate::{
    AiProviderSecret, KeySecret, LEGACY_SYNC_PAYLOAD_VERSION, PlaintextSecrets, ProfileSecret,
    SYNC_PAYLOAD_VERSION, SyncKdf, SyncPayload, SyncPlaintextPayload, WebSearchSecret,
};
use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::profile::SessionProfile;
use miaominal_core::snippet::SnippetRecord;
use miaominal_secrets::{SecretKind, SecretStore};
use miaominal_settings::{AppSettings, SyncedSettings};
use miaominal_storage::SettingsStore;
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::keychain_store::ManagedKeyStore;
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub fn build_payload(
    device_id: &str,
    sessions: &[SessionProfile],
    snippets: &[SnippetRecord],
    managed_keys: &[ManagedKeyRecord],
    settings: &SyncedSettings,
    secret_store: &SecretStore,
    passphrase: &str,
) -> Result<SyncPayload> {
    let synced_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let salt_bytes: [u8; 32] = rand::rng().random();
    let kdf = SyncKdf::argon2id(base64::engine::general_purpose::STANDARD.encode(salt_bytes));
    let plaintext = SyncPlaintextPayload {
        sessions: sessions.to_vec(),
        snippets: snippets.to_vec(),
        managed_keys: managed_keys.to_vec(),
        settings: settings.clone(),
        secrets: collect_secrets(sessions, managed_keys, settings, secret_store)?,
    };

    let mut payload = SyncPayload {
        version: SYNC_PAYLOAD_VERSION,
        device_id: device_id.to_string(),
        synced_at,
        kdf,
        encrypted_payload: String::new(),
    };
    let key = derive_key_for_kdf(passphrase, &payload.kdf)?;
    let plaintext_json =
        serde_json::to_vec(&plaintext).context("failed to serialize sync plaintext")?;
    let aad = associated_data(&payload)?;
    payload.encrypted_payload = encrypt_with_aad(&key, &plaintext_json, &aad)?;

    Ok(payload)
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

pub fn apply_plaintext_payload(
    payload: &SyncPlaintextPayload,
    session_store: &SessionStore,
    snippet_store: &SnippetStore,
    key_store: &ManagedKeyStore,
    secret_store: &SecretStore,
    settings_store: &mut SettingsStore,
    finalize: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let snapshot = PayloadSnapshot::capture(
        payload,
        session_store,
        snippet_store,
        key_store,
        secret_store,
        settings_store,
    )?;

    let apply_result = apply_payload_changes(
        payload,
        session_store,
        snippet_store,
        key_store,
        secret_store,
        settings_store,
    )
    .and_then(|()| finalize().context("failed to finalize sync pull"));

    if let Err(error) = apply_result {
        return match snapshot.restore(
            session_store,
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
    snippet_store: &SnippetStore,
    key_store: &ManagedKeyStore,
    secret_store: &SecretStore,
    settings_store: &mut SettingsStore,
) -> Result<()> {
    let old_sessions = session_store
        .read_sessions_content()?
        .map(|content| session_store.parse_sessions(&content))
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

    session_store.save(&payload.sessions)?;
    snippet_store.save(&payload.snippets)?;
    key_store.save(&payload.managed_keys)?;
    let mut merged_settings = settings_store.settings().clone();
    merged_settings.apply_synced_settings(&payload.settings);
    settings_store.replace(merged_settings)?;
    cleanup_removed_secrets(
        payload,
        &old_sessions,
        &old_keys,
        &old_ai_provider_ids,
        secret_store,
    )?;

    Ok(())
}

#[derive(Debug)]
struct PayloadSnapshot {
    sessions: Vec<SessionProfile>,
    snippets: Vec<SnippetRecord>,
    managed_keys: Vec<ManagedKeyRecord>,
    settings: AppSettings,
    secrets: Vec<SecretSnapshot>,
}

impl PayloadSnapshot {
    fn capture(
        payload: &SyncPlaintextPayload,
        session_store: &SessionStore,
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
        let snippets = snippet_store.load()?;
        let managed_keys = key_store.load()?;
        let settings = settings_store.settings().clone();
        let secrets =
            capture_affected_secrets(payload, &sessions, &managed_keys, &settings, secret_store)?;

        Ok(Self {
            sessions,
            snippets,
            managed_keys,
            settings,
            secrets,
        })
    }

    fn restore(
        self,
        session_store: &SessionStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &mut SettingsStore,
    ) -> Result<()> {
        let mut errors = Vec::new();

        if let Err(error) = session_store.save(&self.sessions) {
            errors.push(format!("sessions: {error:#}"));
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
    }
}

fn decrypt_payload(payload: &SyncPayload, passphrase: &str) -> Result<SyncPlaintextPayload> {
    if payload.version != SYNC_PAYLOAD_VERSION && payload.version != LEGACY_SYNC_PAYLOAD_VERSION {
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
        LEGACY_SYNC_PAYLOAD_VERSION => {
            let legacy: LegacySyncPlaintextPayload = serde_json::from_slice(plaintext_json)
                .context("failed to deserialize legacy decrypted sync payload")?;
            Ok(SyncPlaintextPayload {
                sessions: legacy.sessions,
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

fn associated_data(payload: &SyncPayload) -> Result<Vec<u8>> {
    serde_json::to_vec(&SyncPayloadAssociatedData {
        version: payload.version,
        device_id: &payload.device_id,
        synced_at: payload.synced_at,
        kdf: &payload.kdf,
    })
    .context("failed to serialize sync associated data")
}

fn collect_secrets(
    sessions: &[SessionProfile],
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

    Ok(PlaintextSecrets {
        profile_secrets,
        key_secrets,
        ai_provider_secrets,
        web_search_secret,
    })
}

fn cleanup_removed_secrets(
    payload: &SyncPlaintextPayload,
    old_sessions: &[SessionProfile],
    old_keys: &[ManagedKeyRecord],
    old_ai_provider_ids: &[String],
    secret_store: &SecretStore,
) -> Result<()> {
    let profile_ids: HashSet<&str> = payload
        .sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    for session in old_sessions {
        if !profile_ids.contains(session.id.as_str()) {
            secret_store.delete(&session.id, SecretKind::Password)?;
            secret_store.delete(&session.id, SecretKind::Passphrase)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::keychain::ManagedKeySource;
    use miaominal_secrets::{
        APP_CREDENTIAL_SERVICE, CredentialStore, VaultCredentialBackend, set_vault_test_parameters,
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
            },
        };

        let decrypted = decrypt_payload(
            &legacy_encrypted_payload("correct horse", &plaintext),
            "correct horse",
        )
        .expect("legacy payload should decrypt");

        assert_eq!(decrypted.sessions.len(), 1);
        assert_eq!(decrypted.settings, settings.synced_settings());
        assert_eq!(
            decrypted.secrets.profile_secrets[0].password.as_deref(),
            Some("password")
        );
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
            VaultCredentialBackend::new_with_path(vault_path.clone(), "provider-secret-test"),
        );
        credentials
            .initialize()
            .expect("test credential store should initialize");
        let secret_store = SecretStore::with_credentials(credentials);
        secret_store
            .set("provider-1", SecretKind::AiProviderApiKey, "sk-test")
            .expect("provider api key should save");

        let secrets =
            collect_secrets(&[], &[], &settings, &secret_store).expect("secrets should collect");

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
        let snippet_store = SnippetStore::with_path(root.join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(root.join("managed_keys.toml"));
        let mut settings_store = SettingsStore::load_with_path(root.join("settings.toml"))
            .expect("settings store should load");
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                root.join("secret_vault.json"),
                "vault-passphrase",
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
            },
        }
    }

    fn cleanup_test_vault(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let _ = std::fs::remove_file(std::path::PathBuf::from(lock_path));
    }
}
