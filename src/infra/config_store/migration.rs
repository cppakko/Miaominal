use super::store::SessionStore;
use crate::domain::profile::SessionProfile;
use crate::secrets::{SecretKind, SecretStore};
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
struct LegacyDocument {
    #[serde(default)]
    sessions: Vec<LegacyProfile>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LegacyProfile {
    #[serde(default)]
    id: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    passphrase: String,
}

pub(super) trait SecretWriter {
    fn set_secret(&self, profile_id: &str, kind: SecretKind, value: &str) -> Result<()>;
}

impl SecretWriter for SecretStore {
    fn set_secret(&self, profile_id: &str, kind: SecretKind, value: &str) -> Result<()> {
        self.set(profile_id, kind, value)
    }
}

pub(super) fn load_sessions<S>(store: &SessionStore, secrets: &S) -> Result<Vec<SessionProfile>>
where
    S: SecretWriter + ?Sized,
{
    let Some(content) = store.read_sessions_content()? else {
        return Ok(Vec::new());
    };

    if content.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut sessions = store.parse_sessions(&content)?;
    let migrated = migrate_legacy_plaintext_secrets(&mut sessions, &content, secrets);

    if migrated {
        store.save(&sessions)?;
        log::info!("migrated plaintext secrets from sessions.toml into the OS keyring");
    }

    normalize_auth_methods(&mut sessions);

    Ok(sessions)
}

fn normalize_auth_methods(sessions: &mut [SessionProfile]) {
    for profile in sessions {
        profile.ensure_auth_method();
    }
}

fn migrate_legacy_plaintext_secrets<S>(
    sessions: &mut [SessionProfile],
    content: &str,
    secrets: &S,
) -> bool
where
    S: SecretWriter + ?Sized,
{
    let legacy: LegacyDocument = match toml::from_str(content) {
        Ok(legacy) => legacy,
        Err(error) => {
            log::warn!("failed to parse legacy session secrets: {error:?}");
            return false;
        }
    };

    let mut migrated = false;
    for legacy_profile in legacy.sessions {
        if legacy_profile.password.is_empty() && legacy_profile.passphrase.is_empty() {
            continue;
        }

        let Some(profile) = sessions
            .iter_mut()
            .find(|profile| profile.id == legacy_profile.id)
        else {
            continue;
        };

        if !legacy_profile.password.is_empty() {
            if let Err(error) =
                secrets.set_secret(&profile.id, SecretKind::Password, &legacy_profile.password)
            {
                log::warn!("failed to migrate password for {}: {error:?}", profile.id);
            } else {
                profile.has_stored_password = true;
                migrated = true;
            }
        }

        if !legacy_profile.passphrase.is_empty() {
            if let Err(error) = secrets.set_secret(
                &profile.id,
                SecretKind::Passphrase,
                &legacy_profile.passphrase,
            ) {
                log::warn!("failed to migrate passphrase for {}: {error:?}", profile.id);
            } else {
                profile.has_stored_passphrase = true;
                migrated = true;
            }
        }
    }

    migrated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::profile::AuthMethod;
    use anyhow::{Result, anyhow};
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeSecretStore {
        writes: RefCell<Vec<(String, String, String)>>,
        failing_kind: Option<&'static str>,
    }

    impl SecretWriter for FakeSecretStore {
        fn set_secret(&self, profile_id: &str, kind: SecretKind, value: &str) -> Result<()> {
            let kind = format!("{kind:?}");
            if self.failing_kind == Some(kind.as_str()) {
                return Err(anyhow!("write failed"));
            }

            self.writes
                .borrow_mut()
                .push((profile_id.to_string(), kind, value.to_string()));
            Ok(())
        }
    }

    #[test]
    fn legacy_plaintext_secrets_are_migrated_to_secret_writer() {
        let mut sessions = vec![profile("session-1")];
        let secrets = FakeSecretStore::default();

        let migrated = migrate_legacy_plaintext_secrets(
            &mut sessions,
            r#"
                [[sessions]]
                id = "session-1"
                password = "secret"
                passphrase = "phrase"
            "#,
            &secrets,
        );

        assert!(migrated);
        assert!(sessions[0].has_stored_password);
        assert!(sessions[0].has_stored_passphrase);
        assert_eq!(
            *secrets.writes.borrow(),
            vec![
                (
                    "session-1".to_string(),
                    "Password".to_string(),
                    "secret".to_string()
                ),
                (
                    "session-1".to_string(),
                    "Passphrase".to_string(),
                    "phrase".to_string()
                ),
            ]
        );
    }

    #[test]
    fn failed_legacy_secret_write_does_not_mark_profile_as_stored() {
        let mut sessions = vec![profile("session-1")];
        let secrets = FakeSecretStore {
            failing_kind: Some("Password"),
            ..Default::default()
        };

        let migrated = migrate_legacy_plaintext_secrets(
            &mut sessions,
            r#"
                [[sessions]]
                id = "session-1"
                password = "secret"
            "#,
            &secrets,
        );

        assert!(!migrated);
        assert!(!sessions[0].has_stored_password);
        assert!(secrets.writes.borrow().is_empty());
    }

    #[test]
    fn auth_methods_are_normalized_after_loading() {
        let mut sessions = vec![profile("session-1")];
        sessions[0].private_key_path = "C:/Users/akko/.ssh/id_ed25519".into();

        normalize_auth_methods(&mut sessions);

        assert_eq!(sessions[0].auth_method, Some(AuthMethod::KeyFile));
    }

    fn profile(id: &str) -> SessionProfile {
        let mut profile = SessionProfile::blank(id, 1);
        profile.name = "Legacy".into();
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile.auth_method = None;
        profile
    }
}
