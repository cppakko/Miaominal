use gpui_kit::{App, WeakEntity};
use miaominal_core::snippet::SnippetRecord;
use std::rc::Rc;

use super::{ApplicationState, application_state};
use crate::ui::contracts::snippets::{SnippetCatalog, SnippetCatalogPort, SnippetCommitOperation};

struct ApplicationSnippetCatalog(WeakEntity<ApplicationState>);

impl SnippetCatalog for ApplicationSnippetCatalog {
    fn snapshot(&self, cx: &App) -> Vec<SnippetRecord> {
        self.0
            .upgrade()
            .map(|application| application.read(cx).snippets.clone())
            .unwrap_or_default()
    }

    fn commit(
        &self,
        cx: &mut App,
        operation: SnippetCommitOperation,
    ) -> anyhow::Result<Vec<SnippetRecord>> {
        let application = self
            .0
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("application catalog unavailable"))?;
        application.update(cx, |application, cx| {
            let _sync_guard = miaominal_secrets::lock_sync_data();
            let store = application
                .services
                .snippet_store
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("snippet storage unavailable"))?;
            let candidate = operation(store.load()?)?;
            application.publish_snippets(candidate.clone(), cx);
            Ok(candidate)
        })
    }
}

pub(super) fn port(cx: &App) -> SnippetCatalogPort {
    SnippetCatalogPort::new(Rc::new(ApplicationSnippetCatalog(
        application_state(cx).downgrade(),
    )))
}
