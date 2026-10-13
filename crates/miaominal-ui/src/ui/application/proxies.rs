// Integrate as a private child module of ui::application.
use super::ApplicationState;
use crate::ui::contracts::proxies::{ProxyCommitOperation, ProxyCommitPort, ProxyCommitter};
use anyhow::Result;
use gpui_kit::{App, Entity, WeakEntity};
use miaominal_core::proxy::ProxyProfile;

struct ApplicationProxyCommitter(WeakEntity<ApplicationState>);

impl ProxyCommitter for ApplicationProxyCommitter {
    fn commit(&self, cx: &mut App, operation: ProxyCommitOperation) -> Result<Vec<ProxyProfile>> {
        let _sync_guard = miaominal_secrets::lock_sync_data();
        self.0.update(cx, |application, cx| {
            let store = application
                .services
                .proxy_store
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("proxy store unavailable"))?;
            let profiles = application
                .services
                .session_store
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("session store unavailable"))?
                .load(&application.services.secrets)?;
            let next = operation(store.load(&application.services.secrets)?, profiles)?;
            application.publish_proxies(next.clone(), cx);
            Ok(next)
        })?
    }
}

impl ApplicationState {
    pub(in crate::ui) fn proxy_commit_port(entity: &Entity<Self>) -> ProxyCommitPort {
        ProxyCommitPort::new(ApplicationProxyCommitter(entity.downgrade()))
    }
}
