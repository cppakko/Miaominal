#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretKind {
    Password,
    Passphrase,
    ManagedPrivateKey,
}

impl SecretKind {
    pub(crate) fn suffix(self) -> &'static str {
        match self {
            SecretKind::Password => "password",
            SecretKind::Passphrase => "passphrase",
            SecretKind::ManagedPrivateKey => "managed-private-key",
        }
    }
}
