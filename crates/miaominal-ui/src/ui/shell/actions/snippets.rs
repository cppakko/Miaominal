use super::super::*;
use crate::ui::i18n;

struct SaveSnippetAfterUnlockResult {
    snippets: Vec<SnippetRecord>,
    selected_snippet: Option<usize>,
    description: String,
    persist_error: Option<String>,
}

impl AppView {
    pub(in crate::ui::shell) fn collect_available_snippet_packages(
        snippets: &[SnippetRecord],
    ) -> Vec<String> {
        let mut packages: Vec<_> = snippets
            .iter()
            .filter_map(|snippet| {
                let package = snippet.package.trim();
                (!package.is_empty()).then(|| package.to_string())
            })
            .collect();

        packages.sort_by_key(|package| package.to_ascii_lowercase());
        packages.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        packages
    }

    pub(in crate::ui::shell) fn sync_snippet_package_controls(
        &mut self,
        package: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let package = package.trim();
        let available_packages = Self::collect_available_snippet_packages(&self.data.snippets);
        let selected_existing_package = available_packages
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(package))
            .cloned();

        self.panel_forms
            .snippets
            .package_select
            .update(cx, |select, cx| {
                select.set_items(SearchableVec::new(available_packages.clone()), window, cx);
                if let Some(existing_package) = selected_existing_package.as_ref() {
                    select.set_selected_value(existing_package, window, cx);
                } else {
                    select.set_selected_index(None, window, cx);
                }
            });

        self.panel_forms.snippets.creating_new_package =
            !package.is_empty() && selected_existing_package.is_none();
        set_input_value(
            &self.panel_forms.snippets.package_input,
            if self.panel_forms.snippets.creating_new_package {
                package.to_string()
            } else {
                String::new()
            },
            window,
            cx,
        );
    }

    pub(in crate::ui::shell) fn begin_new_snippet_package(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.panel_forms.snippets.creating_new_package = true;
        self.panel_forms
            .snippets
            .package_select
            .update(cx, |select, cx| {
                select.set_selected_index(None, window, cx);
            });
        set_input_value(&self.panel_forms.snippets.package_input, "", window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn snippet_package_value(&self, cx: &App) -> String {
        if self.panel_forms.snippets.creating_new_package {
            self.panel_forms
                .snippets
                .package_input
                .read(cx)
                .value()
                .trim()
                .to_string()
        } else {
            self.panel_forms
                .snippets
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
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.data.selected_snippet = None;
        self.clear_snippet_inputs(window, cx);
        self.editors.snippets_editor_open = true;
        self.status_message = i18n::string("snippets.messages.preparing_new");
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_existing_snippet_editor(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snippet) = self.data.snippets.get(index).cloned() else {
            return;
        };

        self.data.selected_snippet = Some(index);
        self.populate_snippet_inputs(&snippet, window, cx);
        self.editors.snippets_editor_open = true;
        self.status_message = i18n::string_args(
            "snippets.messages.editing",
            &[("description", &snippet.description)],
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_snippets_editor(&mut self, cx: &mut Context<Self>) {
        if !self.editors.snippets_editor_open {
            return;
        }

        self.editors.snippets_editor_open = false;
        self.data.selected_snippet = None;
        self.status_message = i18n::string("snippets.messages.closed_sidebar");
        cx.notify();
    }

    pub(in crate::ui::shell) fn handle_snippets_package_filter_toggle(
        &mut self,
        package: String,
        cx: &mut Context<Self>,
    ) {
        if self.panel_view.snippets_package_filter.as_deref() == Some(package.as_str()) {
            self.panel_view.snippets_package_filter = None;
        } else {
            self.panel_view.snippets_package_filter = Some(package.clone());
        }

        self.status_message =
            if let Some(selected_package) = self.panel_view.snippets_package_filter.as_ref() {
                i18n::string_args(
                    "snippets.messages.filtering_by_package",
                    &[("package", selected_package)],
                )
            } else {
                i18n::string("snippets.messages.viewing_all")
            };
        cx.notify();
    }

    pub(in crate::ui::shell) fn populate_snippet_inputs(
        &mut self,
        snippet: &SnippetRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(
            &self.panel_forms.snippets.description_input,
            snippet.description.clone(),
            window,
            cx,
        );
        self.sync_snippet_package_controls(&snippet.package, window, cx);
        set_input_value(
            &self.panel_forms.snippets.script_input,
            snippet.script.clone(),
            window,
            cx,
        );
    }

    pub(in crate::ui::shell) fn clear_snippet_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(&self.panel_forms.snippets.description_input, "", window, cx);
        self.sync_snippet_package_controls("", window, cx);
        set_input_value(&self.panel_forms.snippets.script_input, "", window, cx);
    }

    pub(in crate::ui::shell) fn read_snippet_from_inputs(
        &self,
        snippet_id: String,
        cx: &App,
    ) -> Result<SnippetRecord> {
        let description = self
            .panel_forms
            .snippets
            .description_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let package = self.snippet_package_value(cx);
        let script = self
            .panel_forms
            .snippets
            .script_input
            .read(cx)
            .value()
            .to_string();

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
            .data
            .selected_snippet
            .and_then(|index| self.data.snippets.get(index))
            .map(|snippet| snippet.language.clone())
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

    pub(in crate::ui::shell) fn upsert_snippet(&mut self, snippet: SnippetRecord) {
        if let Some(index) = self.data.selected_snippet
            && index < self.data.snippets.len()
        {
            self.data.snippets[index] = snippet;
            return;
        }

        self.data.snippets.push(snippet);
        self.data.selected_snippet = Some(self.data.snippets.len() - 1);
    }

    pub(in crate::ui::shell) fn persist_snippets(&self) -> Result<()> {
        let store =
            self.services.snippet_store.as_ref().ok_or_else(|| {
                anyhow!("snippet config path is not available in this environment")
            })?;
        store.save(&self.data.snippets)
    }

    pub(in crate::ui::shell) fn persist_snippets_after_user_change(
        &mut self,
        _cx: &mut Context<Self>,
    ) -> Result<()> {
        self.persist_snippets()?;
        Ok(())
    }

    fn current_snippet_id(&self) -> String {
        self.data
            .selected_snippet
            .and_then(|index| {
                self.data
                    .snippets
                    .get(index)
                    .map(|snippet| snippet.id.clone())
            })
            .unwrap_or_else(|| self.next_snippet_id())
    }

    fn save_prepared_snippet(
        &mut self,
        snippet: SnippetRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<SnippetRecord> {
        self.upsert_snippet(snippet.clone());

        match self.persist_snippets_after_user_change(cx) {
            Ok(()) => {
                self.populate_snippet_inputs(&snippet, window, cx);
                Ok(snippet)
            }
            Err(_error) if self.services.snippet_store.is_none() => Ok(snippet),
            Err(error) => Err(error),
        }
    }

    pub(in crate::ui::shell) fn next_snippet_id(&self) -> String {
        let mut next = self.data.snippets.len() + 1;
        loop {
            let candidate = format!("snippet-{next}");
            if self
                .data
                .snippets
                .iter()
                .all(|snippet| snippet.id != candidate)
            {
                return candidate;
            }
            next += 1;
        }
    }

    pub(in crate::ui::shell) fn continue_save_snippet_after_unlock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snippet = match self.read_snippet_from_inputs(self.current_snippet_id(), cx) {
            Ok(snippet) => snippet,
            Err(error) => {
                if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
                    self.notify_validation_failure_in_window(
                        window,
                        validation.kind,
                        validation.message.clone(),
                        cx,
                    );
                } else {
                    let message = error.to_string();
                    self.status_message = i18n::string_args(
                        "snippets.messages.save_failed",
                        &[("message", &message)],
                    );
                    cx.notify();
                }
                return;
            }
        };

        let mut snippets = self.data.snippets.clone();
        let selected_snippet = if let Some(index) = self.data.selected_snippet
            && index < snippets.len()
        {
            snippets[index] = snippet.clone();
            Some(index)
        } else {
            snippets.push(snippet.clone());
            Some(snippets.len() - 1)
        };

        let snippet_store = self.services.snippet_store.clone();
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
            self.status_message = i18n::string_args(
                "snippets.messages.save_failed",
                &[("message", &error.to_string())],
            );
            cx.notify();
            return;
        }

        let store_available = self.services.snippet_store.is_some();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv().unwrap_or_else(|_| {
                        Err(anyhow::anyhow!("post-unlock snippet save task cancelled"))
                    })
                })
                .await;

            let _ = this.update(cx, move |this, cx| match result {
                Ok(result) => {
                    this.data.snippets = result.snippets;
                    this.data.selected_snippet = result.selected_snippet;

                    match (store_available, result.persist_error) {
                        (true, None) => {
                            this.editors.snippets_editor_open = false;
                            this.data.selected_snippet = None;
                            this.status_message = i18n::string_args(
                                "snippets.messages.saved",
                                &[("description", result.description.as_str())],
                            );
                        }
                        (false, None) => {
                            this.editors.snippets_editor_open = false;
                            this.data.selected_snippet = None;
                            this.status_message = i18n::string_args(
                                "snippets.messages.saved_memory_only",
                                &[("description", result.description.as_str())],
                            );
                        }
                        (_, Some(error)) => {
                            this.status_message = i18n::string_args(
                                "snippets.messages.save_failed",
                                &[("message", error.as_str())],
                            );
                        }
                    }

                    cx.notify();
                }
                Err(error) => {
                    this.status_message = i18n::string_args(
                        "snippets.messages.save_failed",
                        &[("message", &error.to_string())],
                    );
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(in crate::ui::shell) fn save_snippet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.read_snippet_from_inputs(self.current_snippet_id(), cx) {
            Ok(snippet) => {
                if self.sync_requires_local_vault_unlock() {
                    self.prompt_local_vault_unlock_for_action(
                        PendingLocalVaultUnlockAction::SaveSnippet,
                        window,
                        cx,
                    );
                    return;
                }

                match self.save_prepared_snippet(snippet, window, cx) {
                    Ok(snippet) => {
                        self.editors.snippets_editor_open = false;
                        self.data.selected_snippet = None;
                        self.status_message = if self.services.snippet_store.is_some() {
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
                        cx.notify();
                    }
                    Err(error) => {
                        if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
                            self.notify_validation_failure_in_window(
                                window,
                                validation.kind,
                                validation.message.clone(),
                                cx,
                            );
                        } else {
                            let message = error.to_string();
                            self.status_message = i18n::string_args(
                                "snippets.messages.save_failed",
                                &[("message", &message)],
                            );
                            cx.notify();
                        }
                    }
                }
            }
            Err(error) => {
                if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
                    self.notify_validation_failure_in_window(
                        window,
                        validation.kind,
                        validation.message.clone(),
                        cx,
                    );
                } else {
                    let message = error.to_string();
                    self.status_message = i18n::string_args(
                        "snippets.messages.save_failed",
                        &[("message", &message)],
                    );
                    cx.notify();
                }
            }
        }
    }

    pub(in crate::ui::shell) fn delete_selected_snippet(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.data.selected_snippet else {
            self.status_message = i18n::string("snippets.messages.select_to_delete");
            cx.notify();
            return;
        };

        let Some(snippet) = self.data.snippets.get(index) else {
            self.status_message = i18n::string("snippets.messages.select_to_delete");
            cx.notify();
            return;
        };

        self.dialogs.pending_snippet_delete = Some(PendingSnippetDeleteState {
            snippet_id: snippet.id.clone(),
            snippet_description: snippet.description.clone(),
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn confirm_snippet_delete(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.dialogs.pending_snippet_delete.take() else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::SnippetDelete(pending.clone()), cx);

        let Some(index) = self
            .data
            .snippets
            .iter()
            .position(|snippet| snippet.id == pending.snippet_id)
        else {
            let description = if pending.snippet_description.trim().is_empty() {
                i18n::string("dialogs.snippet_delete.untitled_snippet")
            } else {
                pending.snippet_description.clone()
            };
            self.status_message = i18n::string_args(
                "snippets.messages.already_removed",
                &[("description", description.as_str())],
            );
            cx.notify();
            return;
        };

        self.perform_snippet_delete_at_index(index, cx);
    }

    pub(in crate::ui::shell) fn cancel_snippet_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.dialogs.pending_snippet_delete.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::SnippetDelete(pending), cx);
        }
    }

    fn perform_snippet_delete_at_index(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.data.snippets.len() {
            return;
        }

        let removed = self.data.snippets.remove(index);
        self.data.selected_snippet = None;
        self.editors.snippets_editor_open = false;

        if self
            .panel_view
            .snippets_package_filter
            .as_deref()
            .is_some_and(|filter| filter.eq_ignore_ascii_case(removed.package.as_str()))
            && self.data.snippets.iter().all(|snippet| {
                !snippet
                    .package
                    .eq_ignore_ascii_case(removed.package.as_str())
            })
        {
            self.panel_view.snippets_package_filter = None;
        }

        self.status_message = match self.persist_snippets_after_user_change(cx) {
            Ok(()) => i18n::string_args(
                "snippets.messages.deleted",
                &[("description", &removed.description)],
            ),
            Err(error) => {
                let error = error.to_string();
                i18n::string_args(
                    "snippets.messages.deleted_save_failed",
                    &[("description", &removed.description), ("error", &error)],
                )
            }
        };
        cx.notify();
    }

    pub(in crate::ui::shell) fn handle_workspace_snippets_package_filter_toggle(
        &mut self,
        package: String,
        cx: &mut Context<Self>,
    ) {
        if self
            .workspace_forms
            .snippets_panel
            .selected_package_filter
            .as_deref()
            == Some(package.as_str())
        {
            self.workspace_forms.snippets_panel.selected_package_filter = None;
        } else {
            self.workspace_forms.snippets_panel.selected_package_filter = Some(package.clone());
        }

        self.status_message = if let Some(selected_package) = self
            .workspace_forms
            .snippets_panel
            .selected_package_filter
            .as_ref()
        {
            i18n::string_args(
                "snippets.messages.filtering_by_package",
                &[("package", selected_package)],
            )
        } else {
            i18n::string("snippets.messages.viewing_all")
        };
        cx.notify();
    }
}
