pub mod credential_backend;

mod model;
mod secret_store;

pub use credential_backend::{
    APP_CREDENTIAL_SERVICE, CredentialStore, LockedCredentialBackend, VaultCredentialBackend,
    decrypt_with_aad, encrypt_with_aad,
};
pub use model::SecretKind;
pub use secret_store::SecretStore;
