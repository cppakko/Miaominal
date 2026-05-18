use crate::domain::profile::{ImportedBatch, SessionProfile};
use crate::infra::config_store::store::SessionStore;
use crate::secrets::{SecretKind, SecretStore};
use anyhow::{Result, anyhow};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpsertProfileOutcome {
    Inserted { index: usize },
    Updated { index: usize },
}

#[derive(Debug, Clone)]
pub(crate) struct DeleteProfileOutcome {
    pub removed: SessionProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportedProfilesResult {
    pub imported_count: usize,
    pub warning_count: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct ProfileService {
    session_store: Option<SessionStore>,
    secrets: SecretStore,
}

impl ProfileService {
    pub(crate) fn new(session_store: Option<SessionStore>, secrets: SecretStore) -> Self {
        Self {
            session_store,
            secrets,
        }
    }

    pub(crate) fn parse_tags(tags_text: &str) -> Vec<String> {
        let mut tags = Vec::new();

        for tag in tags_text
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
        {
            if tags
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(tag))
            {
                continue;
            }

            tags.push(tag.to_string());
        }

        tags
    }

    pub(crate) fn commit_profile_secrets(&self, profile: &SessionProfile) -> Result<()> {
        if !profile.password.is_empty() {
            self.secrets
                .set(&profile.id, SecretKind::Password, &profile.password)?;
        }
        if !profile.passphrase.is_empty() {
            self.secrets
                .set(&profile.id, SecretKind::Passphrase, &profile.passphrase)?;
        }
        Ok(())
    }

    pub(crate) fn rollback_imported_profile_secrets(&self, profile_ids: &[String]) {
        for profile_id in profile_ids {
            self.secrets.delete_all(profile_id);
        }
    }

    pub(crate) fn persist_sessions(&self, sessions: &[SessionProfile]) -> Result<()> {
        let store = self
            .session_store
            .as_ref()
            .ok_or_else(|| anyhow!("profile store unavailable"))?;
        store.save(sessions)
    }

    pub(crate) fn next_profile_id(&self, sessions: &[SessionProfile]) -> String {
        let existing_ids: HashSet<&str> =
            sessions.iter().map(|profile| profile.id.as_str()).collect();
        let mut next = sessions.len() + 1;
        loop {
            let candidate = format!("session-{next}");
            if !existing_ids.contains(candidate.as_str()) {
                return candidate;
            }
            next += 1;
        }
    }

    pub(crate) fn upsert_profile(
        &self,
        sessions: &mut Vec<SessionProfile>,
        selected_profile: &mut Option<usize>,
        profile: SessionProfile,
    ) -> UpsertProfileOutcome {
        if let Some(index) = *selected_profile
            && index < sessions.len()
        {
            sessions[index] = profile;
            return UpsertProfileOutcome::Updated { index };
        }

        sessions.push(profile);
        let index = sessions.len() - 1;
        *selected_profile = Some(index);
        UpsertProfileOutcome::Inserted { index }
    }

    pub(crate) fn duplicate_profile(
        &self,
        sessions: &mut Vec<SessionProfile>,
        index: usize,
        duplicate_name: String,
    ) -> Option<SessionProfile> {
        if index >= sessions.len() {
            return None;
        }

        let mut duplicated = sessions[index].clone();
        duplicated.id = self.next_profile_id(sessions);
        duplicated.name = duplicate_name;
        duplicated.is_favorite = false;
        duplicated.last_connected_at = None;

        let insert_at = index + 1;
        sessions.insert(insert_at, duplicated.clone());
        Some(duplicated)
    }

    pub(crate) fn delete_profile(
        &self,
        sessions: &mut Vec<SessionProfile>,
        selected_profile: &mut Option<usize>,
        index: usize,
    ) -> Option<DeleteProfileOutcome> {
        if index >= sessions.len() {
            return None;
        }

        let removed = sessions.remove(index);
        self.secrets.delete_all(&removed.id);

        *selected_profile = match *selected_profile {
            Some(selected) if selected == index => {
                if sessions.is_empty() {
                    None
                } else if index >= sessions.len() {
                    Some(sessions.len() - 1)
                } else {
                    Some(index)
                }
            }
            Some(selected) if selected > index => Some(selected - 1),
            other => other,
        };

        Some(DeleteProfileOutcome { removed })
    }

    pub(crate) fn import_profiles(
        &self,
        existing_sessions: &mut Vec<SessionProfile>,
        batch: ImportedBatch,
    ) -> Result<ImportedProfilesResult> {
        let warning_count = batch.issues.len();
        let imported_count = batch.sessions.len();
        let mut staged_profiles = Vec::with_capacity(imported_count);

        for draft in batch.sessions {
            let profile_id = next_imported_profile_id(existing_sessions, &staged_profiles);
            staged_profiles.push(draft.into_session_profile(profile_id));
        }

        let mut committed_secret_ids = Vec::new();
        for profile in &staged_profiles {
            if let Err(error) = self.commit_profile_secrets(profile) {
                self.rollback_imported_profile_secrets(&committed_secret_ids);
                return Err(error);
            }

            if profile.has_stored_password || profile.has_stored_passphrase {
                committed_secret_ids.push(profile.id.clone());
            }
        }

        let original_len = existing_sessions.len();
        existing_sessions.extend(staged_profiles);
        if let Err(error) = self.persist_sessions(existing_sessions) {
            existing_sessions.truncate(original_len);
            self.rollback_imported_profile_secrets(&committed_secret_ids);
            return Err(error);
        }

        Ok(ImportedProfilesResult {
            imported_count,
            warning_count,
        })
    }
}

fn next_imported_profile_id(
    existing_profiles: &[SessionProfile],
    staged_profiles: &[SessionProfile],
) -> String {
    let existing_ids: HashSet<&str> = existing_profiles
        .iter()
        .map(|profile| profile.id.as_str())
        .chain(staged_profiles.iter().map(|profile| profile.id.as_str()))
        .collect();
    let mut next = existing_profiles.len() + staged_profiles.len() + 1;
    loop {
        let candidate = format!("session-{next}");
        if !existing_ids.contains(candidate.as_str()) {
            return candidate;
        }
        next += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, name: &str) -> SessionProfile {
        let mut profile = SessionProfile::blank(id, 1);
        profile.name = name.to_string();
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile
    }

    #[test]
    fn parse_tags_deduplicates_case_insensitively() {
        assert_eq!(
            ProfileService::parse_tags("prod, staging, PROD, qa"),
            vec!["prod", "staging", "qa"]
        );
    }

    #[test]
    fn next_profile_id_skips_existing_ids() {
        let service = ProfileService::new(None, SecretStore::new_locked_vault());
        let sessions = vec![profile("session-1", "A"), profile("session-3", "B")];

        assert_eq!(service.next_profile_id(&sessions), "session-4");
    }

    #[test]
    fn delete_profile_updates_selected_index() {
        let service = ProfileService::new(None, SecretStore::new_locked_vault());
        let mut sessions = vec![
            profile("session-1", "A"),
            profile("session-2", "B"),
            profile("session-3", "C"),
        ];
        let mut selected = Some(2);

        let outcome = service
            .delete_profile(&mut sessions, &mut selected, 1)
            .expect("profile should be removed");

        assert_eq!(outcome.removed.id, "session-2");
        assert_eq!(selected, Some(1));
        assert_eq!(sessions.len(), 2);
    }
}
