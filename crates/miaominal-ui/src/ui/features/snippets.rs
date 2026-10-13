use super::*;
use crate::ui::contracts::snippets::{SnippetCatalogPort, SnippetSaveSubmission};
use crate::ui::i18n;
use gpui_kit::component::{
    input::{EditorState, InputEvent, TabSize},
    select::{SearchableVec, SelectEvent, SelectState},
};
use gpui_kit::{EventEmitter, Subscription};
use miaominal_core::snippet::SnippetRecord;
use miaominal_storage::config_store::store::SnippetStore;
use std::cell::{Cell, RefCell};
#[path = "snippets/view/mod.rs"]
mod view;
use anyhow::{Result, anyhow};

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingSnippetDeleteState {
    pub(in crate::ui::shell) snippet_id: String,
    pub(in crate::ui::shell) snippet_description: String,
}

#[derive(Clone)]
struct SnippetsForms {
    filter_input: Entity<InputState>,
    description_input: Entity<InputState>,
    package_input: Entity<InputState>,
    package_select: Entity<SelectState<SearchableVec<String>>>,
    creating_new_package: bool,
    script_input: Entity<EditorState>,
}

#[derive(Clone)]
struct SnippetsCatalogViewState {
    snippets_view_mode: ProfileViewMode,
    snippets_package_filter: Option<String>,
}

pub(in crate::ui::shell) struct SnippetsController {
    catalog: SnippetCatalogPort,
    store: Option<SnippetStore>,
    snippets: RefCell<Vec<SnippetRecord>>,
    forms: RefCell<SnippetsForms>,
    editor_id: RefCell<Option<String>>,
    editor_open: Cell<bool>,
    pending_save: RefCell<Option<SnippetSaveSubmission>>,
    editor_revision: Cell<u64>,
    pending_delete: RefCell<Option<PendingSnippetDeleteState>>,
    catalog_view: RefCell<SnippetsCatalogViewState>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<AppCommand> for SnippetsController {}

impl SnippetsController {
    pub(in crate::ui::shell) fn new(
        catalog: SnippetCatalogPort,
        store: Option<SnippetStore>,
        selected: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let snippets = catalog.snapshot(cx);
        let snippet_script_input = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("bash")
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder("")
        });
        set_code_editor_input_placeholder(
            &snippet_script_input,
            i18n::string("placeholders.snippets.script_body"),
            true,
            window,
            cx,
        );

        let forms = SnippetsForms {
            filter_input: new_input_state(
                i18n::string("placeholders.snippets.filter"),
                "",
                false,
                window,
                cx,
            ),
            description_input: new_input_state(
                i18n::string("placeholders.snippets.description_example"),
                "",
                false,
                window,
                cx,
            ),
            package_input: new_input_state(
                i18n::string("placeholders.snippets.new_package_name"),
                "",
                false,
                window,
                cx,
            ),
            package_select: cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(Self::collect_available_snippet_packages(&snippets)),
                    None,
                    window,
                    cx,
                )
            }),
            creating_new_package: snippets.is_empty(),
            script_input: snippet_script_input,
        };
        let subscriptions = vec![
            cx.subscribe(&forms.filter_input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(
                &forms.package_select,
                |controller, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                    let SelectEvent::Confirm(package) = event;
                    if package.is_some() {
                        controller.set_snippets_creating_new_package(false);
                    }
                    cx.notify();
                },
            ),
        ];
        let editor_id =
            selected.and_then(|index| snippets.get(index).map(|snippet| snippet.id.clone()));
        Self {
            catalog,
            store,
            snippets: RefCell::new(snippets),
            forms: RefCell::new(forms),
            editor_id: RefCell::new(editor_id),
            editor_open: Cell::new(false),
            pending_save: RefCell::new(None),
            editor_revision: Cell::new(0),
            pending_delete: RefCell::new(None),
            catalog_view: RefCell::new(SnippetsCatalogViewState {
                snippets_view_mode: ProfileViewMode::Grid,
                snippets_package_filter: None,
            }),
            _subscriptions: subscriptions,
        }
    }

    pub(in crate::ui::shell) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snippets = self.snippets_forms();
        for (input, key) in [
            (&snippets.filter_input, "placeholders.snippets.filter"),
            (
                &snippets.description_input,
                "placeholders.snippets.description_example",
            ),
            (
                &snippets.package_input,
                "placeholders.snippets.new_package_name",
            ),
        ] {
            set_input_placeholder(input, i18n::string(key), window, cx);
        }
        set_code_editor_input_placeholder(
            &snippets.script_input,
            i18n::string("placeholders.snippets.script_body"),
            true,
            window,
            cx,
        );
    }

    pub(in crate::ui::shell) fn editor_open(&self) -> bool {
        self.editor_open.get()
    }
    fn catalog_view(&self) -> SnippetsCatalogViewState {
        self.catalog_view.borrow().clone()
    }

    fn snippets_forms_state(&self) -> &RefCell<SnippetsForms> {
        &self.forms
    }

    fn snippets_forms(&self) -> SnippetsForms {
        self.snippets_forms_state().borrow().clone()
    }

    pub(in crate::ui::shell) fn collect_available_snippet_packages(
        snippets: &[SnippetRecord],
    ) -> Vec<String> {
        let mut packages = snippets
            .iter()
            .filter_map(|snippet| {
                let package = snippet.package.trim();
                (!package.is_empty()).then(|| package.to_string())
            })
            .collect::<Vec<_>>();
        packages.sort_by_key(|package| package.to_ascii_lowercase());
        packages.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        packages
    }

    pub(in crate::ui::shell) fn begin_new_snippet_package(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forms = self.snippets_forms();
        self.set_snippets_creating_new_package(true);
        forms.package_select.update(cx, |select, cx| {
            select.set_selected_index(None, window, cx);
        });
        set_input_value(&forms.package_input, "", window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn dismiss_editor(&self) {
        self.set_snippets_editor_open(false);
        self.set_selected_snippet(None);
    }

    pub(in crate::ui::shell) fn close_snippets_editor(&self, cx: &mut Context<Self>) {
        if !self.editor_open.get() {
            return;
        }
        self.set_snippets_editor_open(false);
        self.set_selected_snippet(None);
        cx.emit(AppCommand::SidebarEditorStateChanged(None));
        cx.emit(AppCommand::Feedback(i18n::string(
            "snippets.messages.closed_sidebar",
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_selected_snippet_delete(&self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_snippet() else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "snippets.messages.select_to_delete",
            )));
            return;
        };
        let Some(snippet) = self.snippets().get(index).cloned() else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "snippets.messages.select_to_delete",
            )));
            return;
        };
        self.set_pending_snippet_delete(Some(PendingSnippetDeleteState {
            snippet_id: snippet.id,
            snippet_description: snippet.description,
        }));
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_snippets_creating_new_package(&self, creating: bool) {
        self.snippets_forms_state()
            .borrow_mut()
            .creating_new_package = creating;
    }

    pub(in crate::ui::shell) fn snippets(&self) -> Vec<SnippetRecord> {
        self.snippets.borrow().clone()
    }

    pub(in crate::ui::shell) fn replace_snippets(&self, snippets: Vec<SnippetRecord>) {
        *self.snippets.borrow_mut() = snippets;
    }

    pub(in crate::ui::shell) fn selected_snippet(&self) -> Option<usize> {
        self.editor_id.borrow().as_ref().and_then(|id| {
            self.snippets
                .borrow()
                .iter()
                .position(|snippet| &snippet.id == id)
        })
    }

    fn set_selected_snippet(&self, selected: Option<usize>) {
        self.editor_id.replace(selected.and_then(|index| {
            self.snippets
                .borrow()
                .get(index)
                .map(|snippet| snippet.id.clone())
        }));
    }

    pub(in crate::ui::shell) fn set_snippets_view_mode(
        &mut self,
        mode: ProfileViewMode,
        cx: &mut Context<Self>,
    ) {
        self.catalog_view.get_mut().snippets_view_mode = mode;
        cx.notify();
    }

    pub(in crate::ui::shell) fn toggle_snippets_package_filter(
        &mut self,
        package: String,
        cx: &mut Context<Self>,
    ) {
        let selected_package = if self
            .catalog_view
            .get_mut()
            .snippets_package_filter
            .as_deref()
            == Some(package.as_str())
        {
            self.catalog_view.get_mut().snippets_package_filter = None;
            None
        } else {
            self.catalog_view.get_mut().snippets_package_filter = Some(package.clone());
            Some(package)
        };
        let status = if let Some(selected_package) = selected_package.as_ref() {
            i18n::string_args(
                "snippets.messages.filtering_by_package",
                &[("package", selected_package)],
            )
        } else {
            i18n::string("snippets.messages.viewing_all")
        };
        cx.emit(AppCommand::Feedback(status));
        cx.notify();
    }

    pub(in crate::ui::shell) fn clear_snippets_package_filter(&self) {
        self.catalog_view.borrow_mut().snippets_package_filter = None;
    }

    fn set_snippets_editor_open(&self, open: bool) {
        self.editor_open.set(open);
        if !open {
            self.advance_editor_revision();
            self.pending_save.take();
        }
    }

    pub(in crate::ui::shell) fn pending_snippet_delete(&self) -> Option<PendingSnippetDeleteState> {
        self.pending_delete.borrow().clone()
    }

    pub(in crate::ui::shell) fn set_pending_snippet_delete(
        &self,
        pending: Option<PendingSnippetDeleteState>,
    ) {
        *self.pending_delete.borrow_mut() = pending;
    }

    pub(in crate::ui::shell) fn take_pending_snippet_delete(
        &self,
    ) -> Option<PendingSnippetDeleteState> {
        self.pending_delete.take()
    }

    pub(in crate::ui::shell) fn confirm_snippet_delete(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_snippet_delete() else {
            return;
        };
        self.delete_snippet_by_id(&pending.snippet_id, &pending.snippet_description, cx);
    }

    pub(in crate::ui::shell) fn cancel_snippet_delete(&self, cx: &mut Context<Self>) {
        if let Some(pending) = self.take_pending_snippet_delete() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::SnippetDelete(pending),
            ));
        }
    }

    fn sync_snippet_package_controls(
        &self,
        package: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let package = package.trim();
        let available_packages = Self::collect_available_snippet_packages(&self.snippets.borrow());
        let selected_existing_package = available_packages
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(package))
            .cloned();
        let forms = self.snippets_forms();

        forms.package_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(available_packages), window, cx);
            if let Some(existing_package) = selected_existing_package.as_ref() {
                select.set_selected_value(existing_package, window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });

        let creating_new_package = !package.is_empty() && selected_existing_package.is_none();
        self.set_snippets_creating_new_package(creating_new_package);
        set_input_value(
            &forms.package_input,
            if creating_new_package {
                package.to_string()
            } else {
                String::new()
            },
            window,
            cx,
        );
    }

    fn snippet_package_value(&self, cx: &App) -> String {
        let forms = self.snippets_forms();
        if forms.creating_new_package {
            forms.package_input.read(cx).value().trim().to_string()
        } else {
            forms
                .package_select
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string()
        }
    }

    pub(in crate::ui::shell) fn open_snippets_editor(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_save.take();
        self.advance_editor_revision();
        self.set_selected_snippet(None);
        self.clear_snippet_inputs(window, cx);
        self.set_snippets_editor_open(true);
        cx.emit(AppCommand::SidebarEditorStateChanged(Some(
            PageEditorSidebarKind::Snippets,
        )));
        cx.emit(AppCommand::Feedback(i18n::string(
            "snippets.messages.preparing_new",
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_existing_snippet_editor(
        &self,
        snippet_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snippet) = self
            .snippets
            .borrow()
            .iter()
            .find(|snippet| snippet.id == snippet_id)
            .cloned()
        else {
            return;
        };

        self.pending_save.take();
        self.advance_editor_revision();
        self.editor_id.replace(Some(snippet.id.clone()));
        self.populate_snippet_inputs(&snippet, window, cx);
        self.set_snippets_editor_open(true);
        cx.emit(AppCommand::SidebarEditorStateChanged(Some(
            PageEditorSidebarKind::Snippets,
        )));
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "snippets.messages.editing",
            &[("description", &snippet.description)],
        )));
        cx.notify();
    }

    fn populate_snippet_inputs(
        &self,
        snippet: &SnippetRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forms = self.snippets_forms();
        set_input_value(
            &forms.description_input,
            snippet.description.clone(),
            window,
            cx,
        );
        self.sync_snippet_package_controls(&snippet.package, window, cx);
        set_editor_value(&forms.script_input, snippet.script.clone(), window, cx);
    }

    fn clear_snippet_inputs(&self, window: &mut Window, cx: &mut Context<Self>) {
        let forms = self.snippets_forms();
        set_input_value(&forms.description_input, "", window, cx);
        self.sync_snippet_package_controls("", window, cx);
        set_editor_value(&forms.script_input, "", window, cx);
    }

    fn read_snippet_from_inputs(&self, snippet_id: String, cx: &App) -> Result<SnippetRecord> {
        let forms = self.snippets_forms();
        let description = forms.description_input.read(cx).value().trim().to_string();
        let package = self.snippet_package_value(cx);
        let script = forms.script_input.read(cx).value().to_string();

        if description.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.snippets.validation.action_description_required",
            ))
            .into());
        }
        if script.trim().is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.snippets.validation.script_required",
            ))
            .into());
        }

        let language = self
            .selected_snippet()
            .and_then(|index| self.snippets.borrow().get(index).cloned())
            .map(|snippet| snippet.language)
            .filter(|language| !language.trim().is_empty())
            .unwrap_or_else(|| "bash".into());

        Ok(SnippetRecord {
            id: snippet_id,
            description,
            package,
            language,
            script,
        })
    }

    fn current_snippet_id(&self) -> String {
        self.editor_id
            .borrow()
            .clone()
            .unwrap_or_else(|| self.next_snippet_id())
    }

    fn save_prepared_snippet(
        &self,
        submission: SnippetSaveSubmission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<SnippetRecord> {
        let is_edit = submission.is_edit();
        let snippet = submission.into_snippet();
        let desired = snippet.clone();
        let store = self.store.clone();
        let next = self.catalog.commit(cx, move |mut latest| {
            if let Some(index) = latest.iter().position(|entry| entry.id == desired.id) {
                latest[index] = desired;
            } else if is_edit {
                return Err(anyhow!(i18n::string("snippets.messages.select_to_delete")));
            } else {
                latest.push(desired);
            }
            let store = store
                .as_ref()
                .ok_or_else(|| anyhow!("snippet store unavailable"))?;
            store.save(&latest)?;
            Ok(latest)
        })?;
        self.replace_snippets(next);
        self.populate_snippet_inputs(&snippet, window, cx);
        Ok(snippet)
    }

    fn next_snippet_id(&self) -> String {
        format!("snippet-{}", uuid::Uuid::new_v4())
    }

    fn report_snippet_save_error(
        &self,
        error: anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
            let message = validation.message.clone();
            cx.emit(AppCommand::Feedback(message.clone()));
            crate::ui::shell::push_app_notification(
                window,
                validation_notification(validation.kind, message),
                cx,
            );
        } else {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "snippets.messages.save_failed",
                &[("message", &error.to_string())],
            )));
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_save_snippet(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.read_snippet_from_inputs(self.current_snippet_id(), cx) {
            Ok(snippet) => cx.emit(AppCommand::SaveSnippetRequested(Box::new(
                self.capture_submission(snippet),
            ))),
            Err(error) => self.report_snippet_save_error(error, window, cx),
        }
    }

    fn advance_editor_revision(&self) {
        self.editor_revision
            .set(self.editor_revision.get().wrapping_add(1).max(1));
    }

    fn capture_submission(&self, snippet: SnippetRecord) -> SnippetSaveSubmission {
        SnippetSaveSubmission::new(
            self.editor_revision.get(),
            snippet,
            self.editor_id.borrow().is_some(),
        )
    }

    fn submission_is_current(&self, submission: &SnippetSaveSubmission) -> bool {
        self.editor_open() && submission.editor_revision() == self.editor_revision.get()
    }

    pub(in crate::ui::shell) fn commit_snippet_save_request(
        &self,
        submission: SnippetSaveSubmission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_submission(submission, window, cx);
    }

    fn commit_submission(
        &self,
        submission: SnippetSaveSubmission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.submission_is_current(&submission) {
            return;
        }
        match self.save_prepared_snippet(submission.clone(), window, cx) {
            Ok(snippet) => {
                self.set_snippets_editor_open(false);
                self.set_selected_snippet(None);
                cx.emit(AppCommand::SidebarEditorStateChanged(None));
                let message = if self.store.is_some() {
                    i18n::string_args(
                        "snippets.messages.saved",
                        &[("description", &snippet.description)],
                    )
                } else {
                    i18n::string_args(
                        "snippets.messages.saved_memory_only",
                        &[("description", &snippet.description)],
                    )
                };
                cx.emit(AppCommand::Feedback(message));
                cx.notify();
            }
            Err(error) => {
                self.pending_save.replace(Some(submission));
                self.report_snippet_save_error(error, window, cx);
            }
        }
    }

    pub(in crate::ui::shell) fn defer_save(&self, submission: SnippetSaveSubmission) -> bool {
        if !self.submission_is_current(&submission) {
            return false;
        }
        self.pending_save.replace(Some(submission));
        true
    }

    pub(in crate::ui::shell) fn continue_save_snippet_after_unlock(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let submission = self.pending_save.take();
        if let Some(submission) = submission {
            self.commit_submission(submission, window, cx);
        }
    }

    pub(in crate::ui::shell) fn delete_snippet_by_id(
        &self,
        snippet_id: &str,
        snippet_description: &str,
        cx: &mut Context<Self>,
    ) {
        let target = snippet_id.to_string();
        let store = self.store.clone();
        let removed_result = std::rc::Rc::new(RefCell::new(None));
        let capture = removed_result.clone();
        let result = self.catalog.commit(cx, move |mut latest| {
            let Some(index) = latest.iter().position(|snippet| snippet.id == target) else {
                return Ok(latest);
            };
            let removed = latest.remove(index);
            let store = store
                .as_ref()
                .ok_or_else(|| anyhow!("snippet store unavailable"))?;
            store.save(&latest)?;
            capture.replace(Some(removed));
            Ok(latest)
        });
        let next = match result {
            Ok(next) => next,
            Err(error) => {
                cx.emit(AppCommand::Feedback(i18n::string_args(
                    "snippets.messages.deleted_save_failed",
                    &[
                        ("description", snippet_description),
                        ("error", &error.to_string()),
                    ],
                )));
                cx.notify();
                return;
            }
        };
        self.replace_snippets(next);
        let Some(removed) = removed_result.take() else {
            self.cancel_snippet_delete(cx);
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "snippets.messages.already_removed",
                &[("description", snippet_description)],
            )));
            cx.notify();
            return;
        };
        self.cancel_snippet_delete(cx);
        if self.editor_id.borrow().as_deref() == Some(snippet_id) {
            self.set_selected_snippet(None);
            self.set_snippets_editor_open(false);
            cx.emit(AppCommand::SidebarEditorStateChanged(None));
        }

        if self
            .catalog_view()
            .snippets_package_filter
            .as_deref()
            .is_some_and(|filter| filter.eq_ignore_ascii_case(removed.package.as_str()))
            && self.snippets.borrow().iter().all(|snippet| {
                !snippet
                    .package
                    .eq_ignore_ascii_case(removed.package.as_str())
            })
        {
            self.clear_snippets_package_filter();
        }

        let message = i18n::string_args(
            "snippets.messages.deleted",
            &[("description", &removed.description)],
        );
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }
}

#[path = "snippets/dialogs.rs"]
mod dialogs;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contracts::snippets::{SnippetCatalog, SnippetCommitOperation};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Render, TestAppContext, WindowBounds, WindowOptions, size};
    use std::rc::Rc;

    #[derive(Clone)]
    struct Catalog(Rc<RefCell<Vec<SnippetRecord>>>, Rc<Cell<usize>>);
    impl SnippetCatalog for Catalog {
        fn snapshot(&self, _: &App) -> Vec<SnippetRecord> {
            self.0.borrow().clone()
        }
        fn commit(
            &self,
            _: &mut App,
            operation: SnippetCommitOperation,
        ) -> anyhow::Result<Vec<SnippetRecord>> {
            let next = operation(self.0.borrow().clone())?;
            if next != *self.0.borrow() {
                self.0.replace(next.clone());
                self.1.set(self.1.get() + 1);
            }
            Ok(next)
        }
    }

    struct Harness {
        controller: Entity<SnippetsController>,
        _subscription: Subscription,
    }
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let save_controller = self.controller.clone();
            div()
                .size_full()
                .child(self.controller.read(cx).render_snippets_editor_sidebar(
                    self.controller.clone(),
                    move |window, cx| {
                        save_controller.update(cx, |controller, cx| {
                            controller.request_save_snippet(window, cx)
                        })
                    },
                    cx,
                ))
        }
    }

    fn subscribe_save_requests(
        controller: &Entity<SnippetsController>,
        window: &mut Window,
        cx: &mut Context<Harness>,
    ) -> Subscription {
        cx.subscribe_in(
            controller,
            window,
            |this: &mut Harness, _, event: &AppCommand, window, cx| {
                if let AppCommand::SaveSnippetRequested(submission) = event {
                    this.controller.update(cx, |controller, cx| {
                        controller.commit_snippet_save_request(
                            submission.as_ref().clone(),
                            window,
                            cx,
                        )
                    });
                }
            },
        )
    }
    fn snippet(id: &str, script: &str) -> SnippetRecord {
        SnippetRecord {
            id: id.into(),
            description: id.into(),
            package: "package".into(),
            language: "bash".into(),
            script: script.into(),
        }
    }

    #[gpui_kit::test]
    fn failed_writes_preserve_drafts_and_deferred_save_uses_captured_id(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let original = vec![snippet("alpha", "original"), snippet("beta", "other")];
        let catalog = Catalog(
            Rc::new(RefCell::new(original.clone())),
            Rc::new(Cell::new(0)),
        );
        let (view, cx) = cx.add_window_view(|window, cx| {
            let controller = cx.new(|cx| {
                SnippetsController::new(
                    SnippetCatalogPort::new(Rc::new(catalog.clone())),
                    Some(SnippetStore::with_path(std::env::temp_dir())),
                    None,
                    window,
                    cx,
                )
            });
            let subscription = subscribe_save_requests(&controller, window, cx);
            Harness {
                controller,
                _subscription: subscription,
            }
        });
        let controller = view.read_with(cx, |view, _| view.controller.clone());
        let submitted = snippet("alpha", "submitted");
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.open_existing_snippet_editor("alpha", window, cx);
                let forms = controller.snippets_forms();
                set_editor_value(&forms.script_input, "draft", window, cx);
                controller.commit_snippet_save_request(
                    controller.capture_submission(submitted.clone()),
                    window,
                    cx,
                );
                assert_eq!(controller.snippets(), original);
                assert!(controller.editor_open());
                assert_eq!(forms.script_input.read(cx).value(), "draft");
                controller.request_selected_snippet_delete(cx);
                controller.confirm_snippet_delete(cx);
                assert_eq!(controller.snippets(), original);
                assert!(controller.pending_snippet_delete().is_some());
                assert_eq!(controller.current_snippet_id(), "alpha");
                controller.cancel_snippet_delete(cx);
                controller.defer_save(controller.capture_submission(submitted.clone()));
                let mut reordered = original.clone();
                reordered.reverse();
                reordered[0].script = "committed by another window".into();
                catalog.0.replace(reordered.clone());
                controller.replace_snippets(reordered);
                controller.continue_save_snippet_after_unlock(window, cx);
                assert!(controller.editor_open());
                assert_eq!(
                    controller.snippets()[0].script,
                    "committed by another window"
                );
                assert_eq!(controller.snippets()[1].script, "original");
                assert_eq!(forms.script_input.read(cx).value(), "draft");
                controller.defer_save(controller.capture_submission(submitted.clone()));
                cx.notify();
            })
        });
        assert_eq!(catalog.1.get(), 0);
        let path = std::env::temp_dir().join(format!(
            "miaominal-snippet-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.store = Some(SnippetStore::with_path(path.clone()));
                controller.continue_save_snippet_after_unlock(window, cx);
                assert!(!controller.editor_open());
                assert_eq!(
                    controller.snippets()[0].script,
                    "committed by another window"
                );
                assert_eq!(controller.snippets()[1], submitted);
                controller.replace_snippets(controller.snippets());
                cx.notify();
            })
        });
        assert_eq!(catalog.1.get(), 1);
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.open_existing_snippet_editor("alpha", window, cx);
                catalog.0.replace(vec![original[1].clone()]);
                controller.replace_snippets(vec![original[1].clone()]);
                assert!(
                    controller
                        .save_prepared_snippet(
                            controller.capture_submission(submitted.clone()),
                            window,
                            cx
                        )
                        .is_err()
                );
                assert_eq!(controller.current_snippet_id(), "alpha");
                controller.defer_save(controller.capture_submission(submitted.clone()));
                controller.open_snippets_editor(window, cx);
                controller.continue_save_snippet_after_unlock(window, cx);
                assert_eq!(controller.snippets(), vec![original[1].clone()]);
                controller.open_existing_snippet_editor("beta", window, cx);
                controller.request_selected_snippet_delete(cx);
                controller.confirm_snippet_delete(cx);
                assert!(controller.snippets().is_empty());
                assert!(controller.pending_snippet_delete().is_none());
                assert!(!controller.editor_open());
            })
        });
        assert_eq!(catalog.1.get(), 2);
        std::fs::remove_file(path).unwrap();
    }

    #[gpui_kit::test]
    fn snippet_editor_validates_and_saves_from_real_keyboard_and_pointer_events(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let catalog = Catalog(Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(0)));
        let path = std::env::temp_dir().join(format!(
            "miaominal-snippet-ui-{}.json",
            uuid::Uuid::new_v4()
        ));
        let (window_handle, harness) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(480.), px(1200.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let controller = cx.new(|cx| {
                            SnippetsController::new(
                                SnippetCatalogPort::new(Rc::new(catalog.clone())),
                                Some(SnippetStore::with_path(path.clone())),
                                None,
                                window,
                                cx,
                            )
                        });
                        controller.update(cx, |controller, cx| {
                            controller.open_snippets_editor(window, cx)
                        });
                        let subscription = subscribe_save_requests(&controller, window, cx);
                        Harness {
                            controller,
                            _subscription: subscription,
                        }
                    })
                },
            )
            .expect("open snippet editor test window")
        });
        let controller = cx.update(|cx| harness.read(cx).controller.clone());
        let forms = cx.update(|cx| controller.read(cx).snippets_forms());
        let description_id = ("input", forms.description_input.entity_id());
        let script_id = ("input", forms.script_input.entity_id());
        let save_id = SharedString::from(format!(
            "editor-button-{}",
            i18n::string("snippets.editor.save")
        ));
        cx.update_window(window_handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(save_id.clone(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(catalog.1.get(), 0);
        assert!(cx.update(|cx| controller.read(cx).editor_open()));
        cx.update_window(window_handle, |_, window, cx| {
            window.click(description_id, cx);
            window.input("部署脚本x", cx);
            window.press("backspace", cx);
            assert_eq!(window.find(description_id).value(), Some("部署脚本"));
            assert_eq!(window.find(description_id).focused(), Some(true));
            window.click(script_id, cx);
            window.input("echo 中文", cx);
            window.press("enter", cx);
            window.input("printf done", cx);
            assert_eq!(
                window.find(script_id).value(),
                Some("echo 中文\nprintf done")
            );
            window.click(save_id.clone(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(catalog.1.get(), 1);
        assert_eq!(catalog.0.borrow()[0].description, "部署脚本");
        assert_eq!(catalog.0.borrow()[0].script, "echo 中文\nprintf done");
        assert!(!cx.update(|cx| controller.read(cx).editor_open()));
        std::fs::remove_file(path).unwrap();
    }

    #[gpui_kit::test]
    fn snippet_replaced_editor_rejects_stale_submission_and_unlock_request(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let original = vec![snippet("alpha", "original"), snippet("beta", "other")];
        let catalog = Catalog(
            Rc::new(RefCell::new(original.clone())),
            Rc::new(Cell::new(0)),
        );
        let (view, cx) = cx.add_window_view(|window, cx| {
            let controller = cx.new(|cx| {
                SnippetsController::new(
                    SnippetCatalogPort::new(Rc::new(catalog.clone())),
                    Some(SnippetStore::with_path(std::env::temp_dir())),
                    None,
                    window,
                    cx,
                )
            });
            let subscription = subscribe_save_requests(&controller, window, cx);
            Harness {
                controller,
                _subscription: subscription,
            }
        });
        let controller = view.read_with(cx, |view, _| view.controller.clone());
        cx.update(|window, cx| {
            controller.update(cx, |controller, cx| {
                controller.open_existing_snippet_editor("alpha", window, cx);
                let stale = controller.capture_submission(snippet("alpha", "submitted"));
                assert!(stale.is_edit());
                assert!(controller.defer_save(stale.clone()));
                controller.open_existing_snippet_editor("beta", window, cx);
                assert!(controller.pending_save.borrow().is_none());
                assert!(!controller.defer_save(stale.clone()));
                controller.commit_snippet_save_request(stale, window, cx);
                assert!(controller.editor_open());
                assert_eq!(controller.current_snippet_id(), "beta");
                assert_eq!(
                    controller.snippets_forms().script_input.read(cx).value(),
                    "other"
                );
                assert!(controller.pending_save.borrow().is_none());
                controller.continue_save_snippet_after_unlock(window, cx);
            })
        });
        assert_eq!(catalog.1.get(), 0);
        assert_eq!(*catalog.0.borrow(), original);
    }
}
