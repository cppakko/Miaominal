use std::rc::Rc;

use gpui_kit::App;
use miaominal_core::snippet::SnippetRecord;

/// A save request captured from one specific editor session.
#[derive(Clone)]
pub(crate) struct SnippetSaveSubmission {
    editor_revision: u64,
    snippet: SnippetRecord,
    is_edit: bool,
}

impl SnippetSaveSubmission {
    pub(crate) fn new(editor_revision: u64, snippet: SnippetRecord, is_edit: bool) -> Self {
        Self {
            editor_revision,
            snippet,
            is_edit,
        }
    }

    pub(crate) fn editor_revision(&self) -> u64 {
        self.editor_revision
    }

    pub(crate) fn is_edit(&self) -> bool {
        self.is_edit
    }

    pub(crate) fn into_snippet(self) -> SnippetRecord {
        self.snippet
    }
}

impl std::fmt::Debug for SnippetSaveSubmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SnippetSaveSubmission")
            .field("editor_revision", &self.editor_revision)
            .field("snippet_id", &self.snippet.id)
            .field("is_edit", &self.is_edit)
            .finish_non_exhaustive()
    }
}

pub(crate) type SnippetCommitOperation =
    Box<dyn FnOnce(Vec<SnippetRecord>) -> anyhow::Result<Vec<SnippetRecord>>>;

/// Access to committed snippet data; mutations persist before source adoption.
pub(crate) trait SnippetCatalog {
    fn snapshot(&self, cx: &App) -> Vec<SnippetRecord>;
    fn commit(
        &self,
        cx: &mut App,
        operation: SnippetCommitOperation,
    ) -> anyhow::Result<Vec<SnippetRecord>>;
}

#[derive(Clone)]
pub(crate) struct SnippetCatalogPort(Rc<dyn SnippetCatalog>);

impl SnippetCatalogPort {
    pub(crate) fn new(catalog: Rc<dyn SnippetCatalog>) -> Self {
        Self(catalog)
    }
    pub(crate) fn snapshot(&self, cx: &App) -> Vec<SnippetRecord> {
        self.0.snapshot(cx)
    }
    pub(crate) fn commit(
        &self,
        cx: &mut App,
        operation: impl FnOnce(Vec<SnippetRecord>) -> anyhow::Result<Vec<SnippetRecord>> + 'static,
    ) -> anyhow::Result<Vec<SnippetRecord>> {
        self.0.commit(cx, Box::new(operation))
    }
}
#[derive(Clone, Debug, Default)]
pub(in crate::ui) struct TransferredSnippetsUi {
    filter: String,
    package: Option<String>,
}

impl TransferredSnippetsUi {
    pub(in crate::ui) fn new(filter: String, package: Option<String>) -> Self {
        Self { filter, package }
    }
    pub(in crate::ui) fn into_parts(self) -> (String, Option<String>) {
        (self.filter, self.package)
    }
}
