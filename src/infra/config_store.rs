#[path = "config_store/import.rs"]
pub(crate) mod import;
#[path = "config_store/migration.rs"]
mod migration;
#[path = "config_store/store.rs"]
pub(crate) mod store;

use self::store::SessionStore;
use crate::domain::profile::SessionProfile;
use crate::secrets::SecretStore;
use anyhow::Result;

impl SessionStore {
    pub fn load(&self, secrets: &SecretStore) -> Result<Vec<SessionProfile>> {
        migration::load_sessions(self, secrets)
    }
}
