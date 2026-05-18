use super::super::*;
use crate::ui::i18n;

fn is_valid_environment_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };

    if first != '_' && !first.is_ascii_alphabetic() {
        return false;
    }

    characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

impl AppView {
    pub(in crate::ui::shell) fn current_host_editor_profile_id(&self) -> Option<&str> {
        self.data
            .selected_profile
            .and_then(|index| self.data.sessions.get(index))
            .map(|profile| profile.id.as_str())
    }

    pub(in crate::ui::shell) fn proxy_jump_candidate_options(
        &self,
        target_profile_id: &str,
    ) -> SearchableVec<ProxyJumpCandidateSelectItem> {
        SearchableVec::new(
            self.available_proxy_jump_candidates(target_profile_id)
                .iter()
                .map(ProxyJumpCandidateSelectItem::new)
                .collect::<Vec<_>>(),
        )
    }

    pub(in crate::ui::shell) fn sync_proxy_jump_candidate_select(
        &mut self,
        selected_profile_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target_profile_id = self
            .current_host_editor_profile_id()
            .unwrap_or("")
            .to_string();
        let options = self.proxy_jump_candidate_options(&target_profile_id);
        let selected_profile_id = selected_profile_id.map(str::to_string);

        self.host_editor_forms
            .proxy_jump_select
            .update(cx, |select, cx| {
                select.set_items(options, window, cx);
                if let Some(selected_profile_id) = selected_profile_id.as_ref() {
                    select.set_selected_value(selected_profile_id, window, cx);
                } else {
                    select.set_selected_index(None, window, cx);
                }
            });
    }

    pub(in crate::ui::shell) fn sync_proxy_jump_candidate_select_in_active_window(
        &mut self,
        selected_profile_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let target_profile_id = self
            .current_host_editor_profile_id()
            .unwrap_or("")
            .to_string();
        let options = self.proxy_jump_candidate_options(&target_profile_id);
        let selected_profile_id = selected_profile_id.map(str::to_string);
        let proxy_jump_select = self.host_editor_forms.proxy_jump_select.clone();

        self.with_active_window(cx, move |window, cx| {
            proxy_jump_select.update(cx, |select, cx| {
                select.set_items(options, window, cx);
                if let Some(selected_profile_id) = selected_profile_id.as_ref() {
                    select.set_selected_value(selected_profile_id, window, cx);
                } else {
                    select.set_selected_index(None, window, cx);
                }
            });
        });
    }

    pub(in crate::ui::shell) fn proxy_jump_chain_profiles(&self) -> Vec<SessionProfile> {
        self.host_editor_forms
            .proxy_jump_profile_ids
            .iter()
            .filter_map(|profile_id| {
                self.data
                    .sessions
                    .iter()
                    .find(|profile| profile.id == *profile_id)
                    .cloned()
            })
            .collect()
    }

    pub(in crate::ui::shell) fn available_proxy_jump_candidates(
        &self,
        target_profile_id: &str,
    ) -> Vec<SessionProfile> {
        let chained_ids: HashSet<&str> = self
            .host_editor_forms
            .proxy_jump_profile_ids
            .iter()
            .map(String::as_str)
            .collect();
        let mut candidates: Vec<_> = self
            .data
            .sessions
            .iter()
            .filter(|profile| {
                profile.id != target_profile_id && !chained_ids.contains(profile.id.as_str())
            })
            .cloned()
            .collect();

        candidates.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        candidates
    }

    pub(in crate::ui::shell) fn add_proxy_jump_profile(
        &mut self,
        profile_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self
            .host_editor_forms
            .proxy_jump_profile_ids
            .iter()
            .any(|existing| existing == profile_id)
        {
            return;
        }

        if self
            .current_host_editor_profile_id()
            .is_some_and(|current_id| current_id == profile_id)
        {
            return;
        }

        self.host_editor_forms
            .proxy_jump_profile_ids
            .push(profile_id.to_string());
        self.host_editor_forms.selected_proxy_jump_hop =
            Some(self.host_editor_forms.proxy_jump_profile_ids.len() - 1);
        self.sync_proxy_jump_candidate_select_in_active_window(None, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn select_proxy_jump_step(
        &mut self,
        step_index: usize,
        cx: &mut Context<Self>,
    ) {
        self.host_editor_forms.selected_proxy_jump_hop =
            if step_index < self.host_editor_forms.proxy_jump_profile_ids.len() {
                Some(step_index)
            } else {
                None
            };
        cx.notify();
    }

    pub(in crate::ui::shell) fn move_selected_proxy_jump_hop_up(&mut self, cx: &mut Context<Self>) {
        let Some(selected_index) = self.host_editor_forms.selected_proxy_jump_hop else {
            return;
        };

        if selected_index == 0 {
            return;
        }

        self.host_editor_forms
            .proxy_jump_profile_ids
            .swap(selected_index - 1, selected_index);
        self.host_editor_forms.selected_proxy_jump_hop = Some(selected_index - 1);
        cx.notify();
    }

    pub(in crate::ui::shell) fn move_selected_proxy_jump_hop_down(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let Some(selected_index) = self.host_editor_forms.selected_proxy_jump_hop else {
            return;
        };

        if selected_index + 1 >= self.host_editor_forms.proxy_jump_profile_ids.len() {
            return;
        }

        self.host_editor_forms
            .proxy_jump_profile_ids
            .swap(selected_index, selected_index + 1);
        self.host_editor_forms.selected_proxy_jump_hop = Some(selected_index + 1);
        cx.notify();
    }

    pub(in crate::ui::shell) fn remove_selected_proxy_jump_hop(&mut self, cx: &mut Context<Self>) {
        let Some(selected_index) = self.host_editor_forms.selected_proxy_jump_hop else {
            return;
        };

        if selected_index >= self.host_editor_forms.proxy_jump_profile_ids.len() {
            self.host_editor_forms.selected_proxy_jump_hop = None;
            return;
        }

        self.host_editor_forms
            .proxy_jump_profile_ids
            .remove(selected_index);
        self.host_editor_forms.selected_proxy_jump_hop =
            if self.host_editor_forms.proxy_jump_profile_ids.is_empty() {
                None
            } else {
                Some(selected_index.min(self.host_editor_forms.proxy_jump_profile_ids.len() - 1))
            };
        self.sync_proxy_jump_candidate_select_in_active_window(None, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn read_proxy_jump_profile_ids(
        &self,
        target_profile_id: &str,
    ) -> Result<Vec<String>> {
        let mut seen = HashSet::new();
        let mut resolved = Vec::new();

        for profile_id in &self.host_editor_forms.proxy_jump_profile_ids {
            let profile_id = profile_id.trim();
            if profile_id.is_empty() {
                continue;
            }

            if profile_id == target_profile_id {
                bail!("host chaining cannot reference the host being edited");
            }

            if !seen.insert(profile_id.to_string()) {
                bail!("host chaining cannot include the same saved host more than once");
            }

            let profile = self
                .data
                .sessions
                .iter()
                .find(|profile| profile.id == profile_id)
                .ok_or_else(|| anyhow!("a selected jump host is no longer available"))?;
            resolved.push(profile.id.clone());
        }

        Ok(resolved)
    }

    pub(in crate::ui::shell) fn host_editor_environment_variable_rows(
        variables: &[SessionEnvironmentVariable],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<HostEditorEnvironmentVariableRow> {
        if variables.is_empty() {
            return vec![Self::new_host_editor_environment_variable_row(
                "", "", window, cx,
            )];
        }

        variables
            .iter()
            .map(|variable| {
                Self::new_host_editor_environment_variable_row(
                    variable.name.clone(),
                    variable.value.clone(),
                    window,
                    cx,
                )
            })
            .collect()
    }

    pub(in crate::ui::shell) fn new_host_editor_environment_variable_row(
        name: impl Into<String>,
        value: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> HostEditorEnvironmentVariableRow {
        HostEditorEnvironmentVariableRow {
            name_input: new_input_state(
                i18n::string("placeholders.host_editor.environment_variable_name"),
                name.into(),
                false,
                window,
                cx,
            ),
            value_input: new_input_state(
                i18n::string("placeholders.host_editor.environment_variable_value"),
                value.into(),
                false,
                window,
                cx,
            ),
        }
    }

    pub(in crate::ui::shell) fn add_environment_variable_row(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.host_editor_forms.environment_variable_rows.push(
            Self::new_host_editor_environment_variable_row("", "", window, cx),
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn remove_environment_variable_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index < self.host_editor_forms.environment_variable_rows.len() {
            self.host_editor_forms
                .environment_variable_rows
                .remove(index);
        }

        if self.host_editor_forms.environment_variable_rows.is_empty() {
            self.host_editor_forms.environment_variable_rows.push(
                Self::new_host_editor_environment_variable_row("", "", window, cx),
            );
        }

        cx.notify();
    }

    pub(in crate::ui::shell) fn read_environment_variables(
        &self,
        cx: &App,
    ) -> Result<Vec<SessionEnvironmentVariable>> {
        let mut variables = Vec::new();

        for (index, row) in self
            .host_editor_forms
            .environment_variable_rows
            .iter()
            .enumerate()
        {
            let raw_name = row.name_input.read(cx).value().to_string();
            let raw_value = row.value_input.read(cx).value().to_string();
            let name = raw_name.trim().to_string();

            if name.is_empty() {
                if raw_value.trim().is_empty() {
                    continue;
                }

                let index = (index + 1).to_string();
                bail!(i18n::string_args(
                    "errors.host_editor.environment_variables.missing_name",
                    &[("index", &index)],
                ));
            }

            if !is_valid_environment_variable_name(&name) {
                bail!(i18n::string_args(
                    "errors.host_editor.environment_variables.invalid_name",
                    &[("name", &name)],
                ));
            }

            variables.push(SessionEnvironmentVariable {
                name,
                value: raw_value,
            });
        }

        Ok(variables)
    }
}
