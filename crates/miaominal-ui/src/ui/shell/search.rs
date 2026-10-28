use super::*;
use crate::ui::contracts::search::{SearchEntry, SearchKind, SearchTarget};
use crate::ui::features::search::SearchEvent;

impl AppView {
    pub(super) fn handle_management_search_event(
        &mut self,
        event: &SearchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SearchEvent::QueryChanged(query) => {
                if let Some(mut tab) = self
                    .shell
                    .search_tab
                    .and_then(|id| self.workspace.tabs.get_mut(id))
                {
                    tab.set_management_query(query.clone());
                } else {
                    self.shell.unattached_search_query = query.clone();
                }
            }
            SearchEvent::Activate(target) => {
                // Resolve again at activation: an open result may have been deleted elsewhere.
                match target {
                    SearchTarget::Setting(id) => {
                        if let Some(destination) =
                            crate::ui::features::settings::search_destination(id)
                        {
                            self.navigate_to_settings_destination(destination, cx);
                        }
                    }
                    SearchTarget::Host(id) => {
                        self.controllers.hosts.update(cx, |controller, cx| {
                            if let Some(index) = controller.profile_index(id) {
                                controller.request_session_at_profile_index(index, cx);
                            }
                        })
                    }
                    SearchTarget::Key(id) => {
                        if self
                            .controllers
                            .keychain
                            .read(cx)
                            .managed_keys()
                            .iter()
                            .any(|key| &key.id == id)
                        {
                            self.set_sidebar_section(SidebarSection::Keychain, cx);
                            self.controllers.keychain.update(cx, |controller, cx| {
                                controller.request_managed_key_rename(id, window, cx)
                            });
                        }
                    }
                    SearchTarget::Snippet(id) => {
                        if self
                            .controllers
                            .snippets
                            .read(cx)
                            .snippets()
                            .iter()
                            .any(|snippet| &snippet.id == id)
                        {
                            self.set_sidebar_section(SidebarSection::Snippets, cx);
                            self.controllers.snippets.update(cx, |controller, cx| {
                                controller.open_existing_snippet_editor(id, window, cx)
                            });
                        }
                    }
                    SearchTarget::Forward { profile, rule } => {
                        let exists =
                            self.controllers
                                .hosts
                                .read(cx)
                                .profiles()
                                .iter()
                                .any(|host| {
                                    &host.id == profile
                                        && host
                                            .port_forwarding_rules
                                            .iter()
                                            .any(|item| &item.id == rule)
                                });
                        if exists {
                            self.set_sidebar_section(SidebarSection::PortForwarding, cx);
                            self.controllers.forwarding.update(cx, |controller, cx| {
                                controller.edit_port_forward_rule(
                                    profile.clone(),
                                    rule.clone(),
                                    window,
                                    cx,
                                )
                            });
                        }
                    }
                    SearchTarget::Trust {
                        host,
                        port,
                        fingerprint,
                    } => {
                        if self
                            .controllers
                            .trusted_hosts
                            .read(cx)
                            .search_snapshot()
                            .iter()
                            .any(|item| {
                                &item.host == host
                                    && item.port == *port
                                    && &item.fingerprint == fingerprint
                            })
                        {
                            self.set_sidebar_section(SidebarSection::KnownHosts, cx);
                            self.controllers.trusted_hosts.update(cx, |controller, cx| {
                                controller.select_trusted_known_host(
                                    host.clone(),
                                    *port,
                                    fingerprint.clone(),
                                    cx,
                                );
                                controller.focus_details(window, cx);
                            });
                        }
                    }
                }
                cx.notify();
            }
        }
    }
    pub(super) fn sync_management_search(
        &mut self,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_id = self.workspace.active_topbar_tab.filter(|id| {
            self.workspace
                .tabs
                .get(*id)
                .is_some_and(|tab| tab.is_hosts())
        });
        if visible && self.shell.search_tab != tab_id {
            let query = tab_id
                .and_then(|id| self.workspace.tabs.get(id))
                .map(|tab| tab.management_query().to_owned())
                .unwrap_or_else(|| self.shell.unattached_search_query.clone());
            self.shell.search_tab = tab_id;
            self.shell
                .search
                .update(cx, |search, cx| search.load_query(query, window, cx));
        }
        if !visible && self.shell.search.read(cx).is_open() {
            self.shell
                .search
                .update(cx, |search, cx| search.dismiss(false, window, cx));
        }
        if self.shell.search_generation == Some(self.application_generations) {
            return;
        }
        self.shell.search_generation = Some(self.application_generations);
        let mut entries = Vec::new();
        for profile in self.controllers.hosts.read(cx).profiles().iter() {
            let category = if profile.is_local() {
                crate::ui::i18n::string("hosts.home.local")
            } else {
                if profile.group.trim().is_empty() {
                    "SSH".into()
                } else {
                    profile.group.clone()
                }
            };
            entries.push(
                SearchEntry::new(
                    format!("host:{}", profile.id),
                    SearchKind::Host,
                    profile.name.clone(),
                    category,
                    format!(
                        "{} {} {} {} {}",
                        profile.host,
                        profile.username,
                        profile.group,
                        profile.tags.join(" "),
                        if profile.is_local() {
                            "local terminal shell 本地终端"
                        } else {
                            "ssh sftp"
                        }
                    ),
                    SearchTarget::Host(profile.id.clone()),
                )
                .with_recent(profile.last_connected_at),
            );
            for rule in &profile.port_forwarding_rules {
                entries.push(SearchEntry::new(
                    format!("forward:{}:{}", profile.id, rule.id),
                    SearchKind::Forward,
                    if rule.label.is_empty() {
                        format!("{}:{}", rule.listen_host, rule.listen_port)
                    } else {
                        rule.label.clone()
                    },
                    profile.name.clone(),
                    format!(
                        "{:?} {} {} {} {}",
                        rule.kind,
                        rule.listen_host,
                        rule.listen_port,
                        rule.target_host,
                        rule.target_port
                    ),
                    SearchTarget::Forward {
                        profile: profile.id.clone(),
                        rule: rule.id.clone(),
                    },
                ));
            }
        }
        for key in self.controllers.keychain.read(cx).managed_keys() {
            entries.push(SearchEntry::new(
                format!("key:{}", key.id),
                SearchKind::Key,
                key.name.clone(),
                key.algorithm.clone(),
                key.id.clone(),
                SearchTarget::Key(key.id.clone()),
            ));
        }
        for snippet in self.controllers.snippets.read(cx).snippets() {
            entries.push(SearchEntry::new(
                format!("snippet:{}", snippet.id),
                SearchKind::Snippet,
                snippet.description.clone(),
                snippet.package.clone(),
                format!("{} {}", snippet.language, snippet.script),
                SearchTarget::Snippet(snippet.id),
            ));
        }
        for item in self.controllers.trusted_hosts.read(cx).search_snapshot() {
            entries.push(SearchEntry::new(
                format!("trust:{}:{}:{}", item.host, item.port, item.fingerprint),
                SearchKind::Trust,
                item.host.clone(),
                item.algorithm.clone(),
                format!("{} {}", item.port, item.fingerprint),
                SearchTarget::Trust {
                    host: item.host,
                    port: item.port,
                    fingerprint: item.fingerprint,
                },
            ));
        }
        entries.extend(crate::ui::features::settings::search_entries());
        self.shell.search.update(cx, |search, cx| {
            search.set_entries(entries);
            cx.notify();
        });
    }
}
