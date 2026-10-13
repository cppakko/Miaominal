// Integrate as a private child module of ui::application.
use super::ApplicationState;
use crate::ui::contracts::keychain::{
    ManagedKeyCommitOperation, ManagedKeyCommitPort, ManagedKeyCommitter, ManagedKeyQueryPort,
};
use anyhow::Result;
use gpui_kit::{App, Entity, WeakEntity};
use miaominal_core::keychain::ManagedKeyRecord;

struct ApplicationManagedKeyCommitter(WeakEntity<ApplicationState>);

impl ManagedKeyCommitter for ApplicationManagedKeyCommitter {
    fn commit(
        &self,
        cx: &mut App,
        operation: ManagedKeyCommitOperation,
    ) -> Result<Vec<ManagedKeyRecord>> {
        let _sync_guard = miaominal_secrets::lock_sync_data();
        self.0.update(cx, |application, cx| {
            let store = application
                .services
                .keychain_store
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("managed key store unavailable"))?;
            let next = operation(store.load()?)?;
            application.publish_managed_keys(next.clone(), cx);
            Ok(next)
        })?
    }
}

impl ApplicationState {
    pub(in crate::ui) fn managed_key_ports(
        entity: &Entity<Self>,
    ) -> (ManagedKeyQueryPort, ManagedKeyCommitPort) {
        let reader = entity.downgrade();
        let writer = ApplicationManagedKeyCommitter(entity.downgrade());
        (
            ManagedKeyQueryPort::new(move |cx| {
                reader
                    .upgrade()
                    .map(|application| application.read(cx).managed_keys.clone())
                    .unwrap_or_default()
            }),
            ManagedKeyCommitPort::new(writer),
        )
    }
}
