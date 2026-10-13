//! Application-owned managed-key catalog access and serialized commit contract.

use anyhow::Result;
use gpui_kit::App;
use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::keychain::ManagedKeySource;
use miaominal_core::profile::SessionProfile;
use miaominal_core::proxy::ProxyProfile;
use std::rc::Rc;

/// Immutable user submission retained while the local vault is unlocked.
#[derive(Clone)]
pub(in crate::ui) struct ManagedKeyImportRequest {
    editor_revision: u64,
    requested_name: String,
    source: ManagedKeySource,
    import_path: Option<String>,
    private_key_material: Option<String>,
    public_key_material: Option<String>,
    passphrase: Option<String>,
    default_import_name: String,
    default_generated_name: String,
}

impl std::fmt::Debug for ManagedKeyImportRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedKeyImportRequest")
            .field("editor_revision", &self.editor_revision)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl ManagedKeyImportRequest {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::ui) fn new(
        editor_revision: u64,
        requested_name: String,
        source: ManagedKeySource,
        import_path: Option<String>,
        private_key_material: Option<String>,
        public_key_material: Option<String>,
        passphrase: Option<String>,
        default_import_name: String,
        default_generated_name: String,
    ) -> Self {
        Self {
            editor_revision,
            requested_name,
            source,
            import_path,
            private_key_material,
            public_key_material,
            passphrase,
            default_import_name,
            default_generated_name,
        }
    }

    pub(in crate::ui) fn editor_revision(&self) -> u64 {
        self.editor_revision
    }
    pub(in crate::ui) fn requested_name(&self) -> &str {
        &self.requested_name
    }
    pub(in crate::ui) fn source(&self) -> ManagedKeySource {
        self.source
    }
    pub(in crate::ui) fn import_path(&self) -> Option<&str> {
        self.import_path.as_deref()
    }
    pub(in crate::ui) fn private_key_material(&self) -> Option<&str> {
        self.private_key_material.as_deref()
    }
    pub(in crate::ui) fn public_key_material(&self) -> Option<&str> {
        self.public_key_material.as_deref()
    }
    pub(in crate::ui) fn passphrase(&self) -> Option<&str> {
        self.passphrase.as_deref()
    }
    pub(in crate::ui) fn default_import_name(&self) -> &str {
        &self.default_import_name
    }
    pub(in crate::ui) fn default_generated_name(&self) -> &str {
        &self.default_generated_name
    }
}

/// The selected key and destination snapshot, never reconstructed from a later editor.
#[derive(Clone, Debug)]
pub(in crate::ui) struct ManagedKeyDeployRequest {
    editor_revision: u64,
    key_summary: String,
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    command: String,
}

impl ManagedKeyDeployRequest {
    pub(in crate::ui) fn new(
        editor_revision: u64,
        key_summary: String,
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
        all_proxies: Vec<ProxyProfile>,
        command: String,
    ) -> Self {
        Self {
            editor_revision,
            key_summary,
            profile,
            all_profiles,
            all_proxies,
            command,
        }
    }

    pub(in crate::ui) fn editor_revision(&self) -> u64 {
        self.editor_revision
    }
    pub(in crate::ui) fn key_summary(&self) -> &str {
        &self.key_summary
    }
    pub(in crate::ui) fn profile(&self) -> &SessionProfile {
        &self.profile
    }
    pub(in crate::ui) fn into_execution(
        self,
    ) -> (
        SessionProfile,
        Vec<SessionProfile>,
        Vec<ProxyProfile>,
        String,
    ) {
        (
            self.profile,
            self.all_profiles,
            self.all_proxies,
            self.command,
        )
    }
}

type ManagedKeyReader = dyn Fn(&App) -> Vec<ManagedKeyRecord>;

#[derive(Clone)]
pub(in crate::ui) struct ManagedKeyQueryPort(Rc<ManagedKeyReader>);

impl ManagedKeyQueryPort {
    pub(in crate::ui) fn new(read: impl Fn(&App) -> Vec<ManagedKeyRecord> + 'static) -> Self {
        Self(Rc::new(read))
    }

    pub(in crate::ui) fn keys(&self, cx: &App) -> Vec<ManagedKeyRecord> {
        (self.0)(cx)
    }
}

pub(in crate::ui) type ManagedKeyCommitOperation =
    Box<dyn FnOnce(Vec<ManagedKeyRecord>) -> Result<Vec<ManagedKeyRecord>>>;

pub(in crate::ui) trait ManagedKeyCommitter {
    fn commit(
        &self,
        cx: &mut App,
        operation: ManagedKeyCommitOperation,
    ) -> Result<Vec<ManagedKeyRecord>>;
}

#[derive(Clone)]
pub(in crate::ui) struct ManagedKeyCommitPort(Rc<dyn ManagedKeyCommitter>);

impl ManagedKeyCommitPort {
    pub(in crate::ui) fn new(committer: impl ManagedKeyCommitter + 'static) -> Self {
        Self(Rc::new(committer))
    }

    pub(in crate::ui) fn commit(
        &self,
        cx: &mut App,
        operation: impl FnOnce(Vec<ManagedKeyRecord>) -> Result<Vec<ManagedKeyRecord>> + 'static,
    ) -> Result<Vec<ManagedKeyRecord>> {
        self.0.commit(cx, Box::new(operation))
    }
}
