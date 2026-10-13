use std::rc::Rc;

use miaominal_core::{profile::SessionProfile, proxy::ProxyProfile};

pub(crate) type ProfileCommitOperation =
    Box<dyn FnOnce(Vec<SessionProfile>) -> anyhow::Result<Vec<SessionProfile>>>;

/// Serializes profile commands against the application catalog. The feature
/// operation must durably persist its candidate before returning success.
pub(crate) trait ProfileCommitter {
    fn commit(
        &self,
        cx: &mut gpui_kit::App,
        operation: ProfileCommitOperation,
    ) -> anyhow::Result<Vec<SessionProfile>>;
}

#[derive(Clone)]
pub(crate) struct ProfileCommitPort(Rc<dyn ProfileCommitter>);

impl ProfileCommitPort {
    pub(crate) fn new(writer: Rc<dyn ProfileCommitter>) -> Self {
        Self(writer)
    }

    /// The operation sees the latest catalog during a synchronous application
    /// update. Failed persistence leaves its source and generation unchanged.
    pub(crate) fn commit(
        &self,
        cx: &mut gpui_kit::App,
        operation: impl FnOnce(Vec<SessionProfile>) -> anyhow::Result<Vec<SessionProfile>> + 'static,
    ) -> anyhow::Result<Vec<SessionProfile>> {
        self.0.commit(cx, Box::new(operation))
    }
}

/// Read access to committed connection data, independent of any window or session.
pub(crate) trait CatalogReader {
    fn profiles(&self) -> Vec<SessionProfile>;
    fn profile(&self, id: &str) -> Option<SessionProfile>;
    fn proxies(&self) -> Vec<ProxyProfile>;
}

/// An application-owned catalog reader. Clones retain the same source across reloads.
#[derive(Clone)]
pub(crate) struct CatalogQueryPort(Rc<dyn CatalogReader>);

impl CatalogQueryPort {
    pub(crate) fn new(reader: Rc<dyn CatalogReader>) -> Self {
        Self(reader)
    }

    pub(crate) fn profiles(&self) -> Vec<SessionProfile> {
        self.0.profiles()
    }

    pub(crate) fn profile(&self, id: &str) -> Option<SessionProfile> {
        self.0.profile(id)
    }

    pub(crate) fn proxies(&self) -> Vec<ProxyProfile> {
        self.0.proxies()
    }
}
