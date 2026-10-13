#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum LocalVaultStatus {
    Disabled,
    Locked,
    Unlocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum LocalVaultPassphrasePopupMode {
    PrimaryAction,
    ChangePassphrase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) enum SecretRevealTarget {
    SyncGithubToken,
    SyncWebdavPassword,
    SyncPassphrase,
    SyncPassphraseConfirmation,
    LocalVaultCurrentPassphrase,
    LocalVaultPassphrase,
    LocalVaultPassphraseConfirmation,
    AiProviderApiKey(String),
    WebSearchApiKey,
}

impl SecretRevealTarget {
    pub(in crate::ui) fn uses_stored_secret(&self) -> bool {
        matches!(
            self,
            Self::SyncGithubToken
                | Self::SyncWebdavPassword
                | Self::AiProviderApiKey(_)
                | Self::WebSearchApiKey
        )
    }
}
