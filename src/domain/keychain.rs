use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ManagedKeySource {
    #[default]
    Generated,
    Imported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedKeyRecord {
    pub id: String,
    pub name: String,
    pub algorithm: String,
    pub public_key: String,
    pub source: ManagedKeySource,
}

impl ManagedKeyRecord {
    pub fn summary(&self) -> String {
        format!("{} ({})", self.name, self.algorithm)
    }
}
