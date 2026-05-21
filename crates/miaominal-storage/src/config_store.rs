#[path = "config_store/import.rs"]
pub mod import;
#[path = "config_store/migration.rs"]
mod migration;
#[path = "config_store/store.rs"]
pub mod store;

use self::store::SessionStore;
use anyhow::Result;
use miaominal_core::profile::SessionProfile;
use miaominal_secrets::SecretStore;

impl SessionStore {
    pub fn load(&self, secrets: &SecretStore) -> Result<Vec<SessionProfile>> {
        migration::load_sessions(self, secrets)
    }
}
