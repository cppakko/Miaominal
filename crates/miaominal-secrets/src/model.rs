#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretKind {
    Password,
    Passphrase,
    ManagedPrivateKey,
    AiProviderApiKey,
}

impl SecretKind {
    pub(crate) fn suffix(self) -> &'static str {
        match self {
            SecretKind::Password => "password",
            SecretKind::Passphrase => "passphrase",
            SecretKind::ManagedPrivateKey => "managed-private-key",
            SecretKind::AiProviderApiKey => "ai-provider-api-key",
        }
    }
}
