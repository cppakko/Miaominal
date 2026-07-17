use super::*;
use anyhow::{Result, anyhow};

struct SaveSnippetAfterUnlockResult {
    snippets: Vec<SnippetRecord>,
    selected_snippet: Option<usize>,
    description: String,
    persist_error: Option<String>,
}

impl SessionController {
    pub(in crate::ui::shell) fn confirm_snippet_delete(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.take_pending_snippet_delete() else {
            return;
        };
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::SnippetDelete(pending.clone()),
        ));
        self.delete_snippet_by_id(&pending.snippet_id, &pending.snippet_description, cx);
    }

    pub(in crate::ui::shell) fn cancel_snippet_delete(&mut self, cx: &mut Context<Self>) {
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
        self.set_selected_snippet(None);
        self.clear_snippet_inputs(window, cx);
        self.set_snippets_editor_open(true);
        cx.emit(AppCommand::Feedback(i18n::string(
            "snippets.messages.preparing_new",
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_existing_snippet_editor(
        &self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snippet) = self.snippets.borrow().get(index).cloned() else {
            return;
        };

        self.set_selected_snippet(Some(index));
        self.populate_snippet_inputs(&snippet, window, cx);
        self.set_snippets_editor_open(true);
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
        set_input_value(&forms.script_input, snippet.script.clone(), window, cx);
    }

    fn clear_snippet_inputs(&self, window: &mut Window, cx: &mut Context<Self>) {
        let forms = self.snippets_forms();
        set_input_value(&forms.description_input, "", window, cx);
        self.sync_snippet_package_controls("", window, cx);
        set_input_value(&forms.script_input, "", window, cx);
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

    fn upsert_snippet(&self, snippet: SnippetRecord) {
        let mut snippets = self.snippets.borrow_mut();
        if let Some(index) = self.selected_snippet()
            && index < snippets.len()
        {
            snippets[index] = snippet;
            return;
        }

        snippets.push(snippet);
        self.set_selected_snippet(Some(snippets.len() - 1));
    }

    fn persist_snippets(&self) -> Result<()> {
        let store =
            self.services.snippet_store.clone().ok_or_else(|| {
                anyhow!("snippet config path is not available in this environment")
            })?;
        store.save(&self.snippets.borrow())
    }

    fn current_snippet_id(&self) -> String {
        self.selected_snippet()
            .and_then(|index| {
                self.snippets
                    .borrow()
                    .get(index)
                    .map(|snippet| snippet.id.clone())
            })
            .unwrap_or_else(|| self.next_snippet_id())
    }

    fn save_prepared_snippet(
        &self,
        snippet: SnippetRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<SnippetRecord> {
        self.upsert_snippet(snippet.clone());

        match self.persist_snippets() {
            Ok(()) => {
                self.populate_snippet_inputs(&snippet, window, cx);
                Ok(snippet)
            }
            Err(_error) if self.services.snippet_store.is_none() => Ok(snippet),
            Err(error) => Err(error),
        }
    }

    fn next_snippet_id(&self) -> String {
        let snippets = self.snippets.borrow();
        let mut next = snippets.len() + 1;
        loop {
            let candidate = format!("snippet-{next}");
            if snippets.iter().all(|snippet| snippet.id != candidate) {
                return candidate;
            }
            next += 1;
        }
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
            window.push_notification(validation_notification(validation.kind, message), cx);
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
            Ok(snippet) => cx.emit(AppCommand::SaveSnippetRequested(Box::new(snippet))),
            Err(error) => self.report_snippet_save_error(error, window, cx),
        }
    }

    pub(in crate::ui::shell) fn commit_snippet_save_request(
        &self,
        snippet: SnippetRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.save_prepared_snippet(snippet, window, cx) {
            Ok(snippet) => {
                self.set_snippets_editor_open(false);
                self.set_selected_snippet(None);
                let message = if self.services.snippet_store.is_some() {
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
            Err(error) => self.report_snippet_save_error(error, window, cx),
        }
    }

    pub(in crate::ui::shell) fn continue_save_snippet_after_unlock(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snippet = match self.read_snippet_from_inputs(self.current_snippet_id(), cx) {
            Ok(snippet) => snippet,
            Err(error) => {
                self.report_snippet_save_error(error, window, cx);
                return;
            }
        };

        let mut snippets = self.snippets.borrow().clone();
        let selected_snippet = if let Some(index) = self.selected_snippet()
            && index < snippets.len()
        {
            snippets[index] = snippet.clone();
            Some(index)
        } else {
            snippets.push(snippet.clone());
            Some(snippets.len() - 1)
        };

        let snippet_store = self.services.snippet_store.clone();
        let store_available = snippet_store.is_some();
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<SaveSnippetAfterUnlockResult>>(1);
        let spawn_result = std::thread::Builder::new()
            .name("post-unlock-snippet-save".to_string())
            .spawn(move || {
                let persist_error = snippet_store
                    .map(|store| store.save(&snippets).err().map(|error| error.to_string()))
                    .unwrap_or(None);
                tx.send(Ok(SaveSnippetAfterUnlockResult {
                    snippets,
                    selected_snippet,
                    description: snippet.description,
                    persist_error,
                }))
                .ok();
            });

        if let Err(error) = spawn_result {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "snippets.messages.save_failed",
                &[("message", &error.to_string())],
            )));
            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow!("post-unlock snippet save task cancelled")))
                })
                .await;

            let _ = this.update(cx, move |controller, cx| match result {
                Ok(result) => {
                    controller.replace_snippets(result.snippets);
                    controller.set_selected_snippet(result.selected_snippet);

                    let message = match (store_available, result.persist_error) {
                        (true, None) => {
                            controller.set_snippets_editor_open(false);
                            controller.set_selected_snippet(None);
                            i18n::string_args(
                                "snippets.messages.saved",
                                &[("description", result.description.as_str())],
                            )
                        }
                        (false, None) => {
                            controller.set_snippets_editor_open(false);
                            controller.set_selected_snippet(None);
                            i18n::string_args(
                                "snippets.messages.saved_memory_only",
                                &[("description", result.description.as_str())],
                            )
                        }
                        (_, Some(error)) => i18n::string_args(
                            "snippets.messages.save_failed",
                            &[("message", error.as_str())],
                        ),
                    };
                    cx.emit(AppCommand::Feedback(message));
                    cx.notify();
                }
                Err(error) => {
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "snippets.messages.save_failed",
                        &[("message", &error.to_string())],
                    )));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(in crate::ui::shell) fn delete_snippet_by_id(
        &self,
        snippet_id: &str,
        snippet_description: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .snippets
            .borrow()
            .iter()
            .position(|snippet| snippet.id == snippet_id)
        else {
            let description = if snippet_description.trim().is_empty() {
                i18n::string("dialogs.snippet_delete.untitled_snippet")
            } else {
                snippet_description.to_string()
            };
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "snippets.messages.already_removed",
                &[("description", description.as_str())],
            )));
            cx.notify();
            return;
        };

        let removed = self.snippets.borrow_mut().remove(index);
        self.set_selected_snippet(None);
        self.set_snippets_editor_open(false);

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

        let message = match self.persist_snippets() {
            Ok(()) => i18n::string_args(
                "snippets.messages.deleted",
                &[("description", &removed.description)],
            ),
            Err(error) => i18n::string_args(
                "snippets.messages.deleted_save_failed",
                &[
                    ("description", &removed.description),
                    ("error", &error.to_string()),
                ],
            ),
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn handle_workspace_snippets_package_filter_toggle(
        &self,
        package: String,
        cx: &mut Context<Self>,
    ) {
        let selected_package = self.toggle_workspace_snippets_package_filter(&package);
        let message = if let Some(selected_package) = selected_package.as_ref() {
            i18n::string_args(
                "snippets.messages.filtering_by_package",
                &[("package", selected_package)],
            )
        } else {
            i18n::string("snippets.messages.viewing_all")
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }
}
