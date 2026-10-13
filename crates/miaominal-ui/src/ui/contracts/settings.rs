//! Immutable Settings submissions retained across vault unlock and worker completion.

use crate::ui::contracts::security::SecretRevealTarget;
use gpui_kit::App;
use miaominal_core::proxy::ProxyProfile;
use miaominal_secrets::ProtectedPassphrase;
use miaominal_services::ProxyPasswordUpdate;
use miaominal_settings::{AiProviderConfig, WebSearchConfig};
use miaominal_sync::SyncProvider;
use std::rc::Rc;

/// Publishes durable Settings changes independently of the window that submitted them.
#[derive(Clone)]
pub(in crate::ui) struct SettingsPersistencePort(Rc<dyn Fn(&mut App)>);

impl SettingsPersistencePort {
    pub(in crate::ui) fn new(publish: impl Fn(&mut App) + 'static) -> Self {
        Self(Rc::new(publish))
    }

    pub(in crate::ui) fn published(&self, cx: &mut App) {
        (self.0)(cx);
    }
}

#[derive(Clone)]
pub(in crate::ui) struct AiProviderSaveDraft {
    provider: AiProviderConfig,
    api_key: String,
    editor_revision: u64,
    requires_existing_target: bool,
}

impl AiProviderSaveDraft {
    pub(in crate::ui) fn new(
        provider: AiProviderConfig,
        api_key: String,
        requires_existing_target: bool,
    ) -> Self {
        Self {
            provider,
            api_key,
            editor_revision: 0,
            requires_existing_target,
        }
    }
    pub(in crate::ui) fn with_editor_revision(mut self, revision: u64) -> Self {
        self.editor_revision = revision;
        self
    }
    pub(in crate::ui) fn api_key(&self) -> &str {
        &self.api_key
    }

    pub(in crate::ui) fn editor_revision(&self) -> u64 {
        self.editor_revision
    }
    pub(in crate::ui) fn into_parts(self) -> (AiProviderConfig, String, u64, bool) {
        (
            self.provider,
            self.api_key,
            self.editor_revision,
            self.requires_existing_target,
        )
    }
}

#[derive(Clone, Debug)]
pub(in crate::ui) struct SecretRevealRequest {
    target: SecretRevealTarget,
    editor_revision: u64,
    surface_key: &'static str,
}
impl SecretRevealRequest {
    pub(in crate::ui) fn new(
        target: SecretRevealTarget,
        surface_key: &'static str,
        editor_revision: u64,
    ) -> Self {
        Self {
            target,
            surface_key,
            editor_revision,
        }
    }

    pub(in crate::ui) fn surface_key(&self) -> &'static str {
        self.surface_key
    }
    pub(in crate::ui) fn editor_revision(&self) -> u64 {
        self.editor_revision
    }
    pub(in crate::ui) fn into_target(self) -> SecretRevealTarget {
        self.target
    }
}

impl std::fmt::Debug for AiProviderSaveDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiProviderSaveDraft")
            .field("provider_id", &self.provider.id)
            .field("editor_revision", &self.editor_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(in crate::ui) struct WebSearchSaveDraft {
    config: WebSearchConfig,
    api_key: String,
    editor_revision: u64,
}

impl WebSearchSaveDraft {
    pub(in crate::ui) fn new(config: WebSearchConfig, api_key: String) -> Self {
        Self {
            config,
            api_key,
            editor_revision: 0,
        }
    }
    pub(in crate::ui) fn with_editor_revision(mut self, revision: u64) -> Self {
        self.editor_revision = revision;
        self
    }
    pub(in crate::ui) fn api_key(&self) -> &str {
        &self.api_key
    }

    pub(in crate::ui) fn editor_revision(&self) -> u64 {
        self.editor_revision
    }
    pub(in crate::ui) fn into_parts(self) -> (WebSearchConfig, String, u64) {
        (self.config, self.api_key, self.editor_revision)
    }
}

impl std::fmt::Debug for WebSearchSaveDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSearchSaveDraft")
            .field("kind", &self.config.kind)
            .field("editor_revision", &self.editor_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(in crate::ui) struct ProxySaveDraft {
    proxy: ProxyProfile,
    password_update: ProxyPasswordUpdate,
    editor_revision: u64,
    requires_existing_target: bool,
}

impl ProxySaveDraft {
    pub(in crate::ui) fn new(
        proxy: ProxyProfile,
        password_update: ProxyPasswordUpdate,
        requires_existing_target: bool,
    ) -> Self {
        Self {
            proxy,
            password_update,
            editor_revision: 0,
            requires_existing_target,
        }
    }
    pub(in crate::ui) fn with_editor_revision(mut self, revision: u64) -> Self {
        self.editor_revision = revision;
        self
    }
    pub(in crate::ui) fn proxy(&self) -> &ProxyProfile {
        &self.proxy
    }
    pub(in crate::ui) fn editor_revision(&self) -> u64 {
        self.editor_revision
    }
    pub(in crate::ui) fn into_parts(self) -> (ProxyProfile, ProxyPasswordUpdate, bool) {
        (
            self.proxy,
            self.password_update,
            self.requires_existing_target,
        )
    }
}

impl std::fmt::Debug for ProxySaveDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxySaveDraft")
            .field("proxy_id", &self.proxy.id)
            .field("editor_revision", &self.editor_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(in crate::ui) struct SyncProviderConfigSaveDraft {
    provider: SyncProvider,
    editor_revision: u64,
    token: String,
    gist_id: Option<String>,
    url: String,
    username: String,
    password: String,
}

impl SyncProviderConfigSaveDraft {
    pub(in crate::ui) fn github_gist(
        token: String,
        gist_id: Option<String>,
        revision: u64,
    ) -> Self {
        Self {
            provider: SyncProvider::GithubGist,
            editor_revision: revision,
            token,
            gist_id,
            url: String::new(),
            username: String::new(),
            password: String::new(),
        }
    }
    pub(in crate::ui) fn webdav(
        url: String,
        username: String,
        password: String,
        revision: u64,
    ) -> Self {
        Self {
            provider: SyncProvider::WebDav,
            editor_revision: revision,
            token: String::new(),
            gist_id: None,
            url,
            username,
            password,
        }
    }
    pub(in crate::ui) fn provider(&self) -> SyncProvider {
        self.provider
    }
    pub(in crate::ui) fn editor_revision(&self) -> u64 {
        self.editor_revision
    }
    pub(in crate::ui) fn into_parts(self) -> (String, Option<String>, String, String, String) {
        (
            self.token,
            self.gist_id,
            self.url,
            self.username,
            self.password,
        )
    }
}

impl std::fmt::Debug for SyncProviderConfigSaveDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncProviderConfigSaveDraft")
            .field("provider", &self.provider)
            .field("editor_revision", &self.editor_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(in crate::ui) struct SyncPassphraseSaveDraft {
    passphrase: String,
    editor_revision: u64,
}

impl SyncPassphraseSaveDraft {
    pub(in crate::ui) fn new(passphrase: String, revision: u64) -> Self {
        Self {
            passphrase,
            editor_revision: revision,
        }
    }
    pub(in crate::ui) fn editor_revision(&self) -> u64 {
        self.editor_revision
    }
    pub(in crate::ui) fn into_passphrase(self) -> String {
        self.passphrase
    }
}

impl std::fmt::Debug for SyncPassphraseSaveDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncPassphraseSaveDraft")
            .field("editor_revision", &self.editor_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui) enum LocalVaultActionKind {
    Enable,
    Unlock,
    Lock,
    Disable,
    ChangePassphrase,
}

#[derive(Clone)]
pub(in crate::ui) struct LocalVaultActionRequest {
    kind: LocalVaultActionKind,
    passphrase: Option<ProtectedPassphrase>,
    new_passphrase: Option<ProtectedPassphrase>,
}

impl LocalVaultActionRequest {
    pub(in crate::ui) fn enable(passphrase: ProtectedPassphrase) -> Self {
        Self {
            kind: LocalVaultActionKind::Enable,
            passphrase: Some(passphrase),
            new_passphrase: None,
        }
    }
    pub(in crate::ui) fn unlock(passphrase: ProtectedPassphrase) -> Self {
        Self {
            kind: LocalVaultActionKind::Unlock,
            passphrase: Some(passphrase),
            new_passphrase: None,
        }
    }
    pub(in crate::ui) fn lock() -> Self {
        Self {
            kind: LocalVaultActionKind::Lock,
            passphrase: None,
            new_passphrase: None,
        }
    }
    pub(in crate::ui) fn disable() -> Self {
        Self {
            kind: LocalVaultActionKind::Disable,
            passphrase: None,
            new_passphrase: None,
        }
    }
    pub(in crate::ui) fn change_passphrase(
        current: ProtectedPassphrase,
        new: ProtectedPassphrase,
    ) -> Self {
        Self {
            kind: LocalVaultActionKind::ChangePassphrase,
            passphrase: Some(current),
            new_passphrase: Some(new),
        }
    }
    pub(in crate::ui) fn into_parts(
        self,
    ) -> (
        LocalVaultActionKind,
        Option<ProtectedPassphrase>,
        Option<ProtectedPassphrase>,
    ) {
        (self.kind, self.passphrase, self.new_passphrase)
    }
}

impl std::fmt::Debug for LocalVaultActionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalVaultActionRequest")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
