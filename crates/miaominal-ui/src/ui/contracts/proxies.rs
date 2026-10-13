//! Serialized mutation of the application-owned proxy catalog.

use anyhow::Result;
use gpui_kit::App;
use miaominal_core::profile::SessionProfile;
use miaominal_core::proxy::ProxyProfile;
use std::rc::Rc;

pub(in crate::ui) type ProxyCommitOperation =
    Box<dyn FnOnce(Vec<ProxyProfile>, Vec<SessionProfile>) -> Result<Vec<ProxyProfile>>>;

pub(in crate::ui) trait ProxyCommitter {
    fn commit(&self, cx: &mut App, operation: ProxyCommitOperation) -> Result<Vec<ProxyProfile>>;
}

#[derive(Clone)]
pub(in crate::ui) struct ProxyCommitPort(Rc<dyn ProxyCommitter>);

impl ProxyCommitPort {
    pub(in crate::ui) fn new(committer: impl ProxyCommitter + 'static) -> Self {
        Self(Rc::new(committer))
    }

    pub(in crate::ui) fn commit(
        &self,
        cx: &mut App,
        operation: impl FnOnce(Vec<ProxyProfile>, Vec<SessionProfile>) -> Result<Vec<ProxyProfile>>
        + 'static,
    ) -> Result<Vec<ProxyProfile>> {
        self.0.commit(cx, Box::new(operation))
    }
}
