use std::rc::Rc;

use miaominal_core::{profile::SessionProfile, proxy::ProxyProfile};

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
