pub mod credential_backend;

mod model;
mod protected_memory;
mod secret_store;

pub use credential_backend::{
    APP_CREDENTIAL_SERVICE, CredentialStore, LockedCredentialBackend, VaultCredentialBackend,
    decrypt_with_aad, encrypt_with_aad, set_vault_test_parameters,
};
pub use model::SecretKind;
pub use protected_memory::{MAX_VAULT_PASSPHRASE_BYTES, ProtectedPassphrase};
pub use secret_store::SecretStore;
