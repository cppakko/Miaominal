//! UI search contract. Contains public presentation data and searchable text, never vault credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::ui) enum SearchKind {
    Host,
    Key,
    Forward,
    Snippet,
    Trust,
    Setting,
}
impl SearchKind {
    pub(in crate::ui) fn label_key(self) -> &'static str {
        match self {
            Self::Host => "search.hosts",
            Self::Key => "search.keys",
            Self::Forward => "search.forwards",
            Self::Snippet => "search.snippets",
            Self::Trust => "search.trust",
            Self::Setting => "search.settings",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) enum SearchTarget {
    Setting(&'static str),
    Host(String),
    Key(String),
    Forward {
        profile: String,
        rule: String,
    },
    Snippet(String),
    Trust {
        host: String,
        port: u16,
        fingerprint: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) struct SearchEntry {
    id: String,
    kind: SearchKind,
    title: String,
    category: String,
    terms: String,
    target: SearchTarget,
    recent: Option<u64>,
}
impl SearchEntry {
    pub(in crate::ui) fn new(
        id: String,
        kind: SearchKind,
        title: String,
        category: String,
        terms: String,
        target: SearchTarget,
    ) -> Self {
        let aliases = match kind {
            SearchKind::Host => "host profile 主机 配置",
            SearchKind::Key => "key keychain 密钥",
            SearchKind::Forward => "port forwarding forward tunnel profile 端口转发 隧道 配置",
            SearchKind::Snippet => "snippet script 脚本",
            SearchKind::Trust => "known trusted host 信任记录",
            SearchKind::Setting => "settings preferences 设置",
        };
        let terms = format!("{title} {category} {terms} {aliases}").to_lowercase();
        Self {
            id,
            kind,
            title,
            category,
            terms,
            target,
            recent: None,
        }
    }
    pub(in crate::ui) fn with_recent(mut self, recent: Option<u64>) -> Self {
        self.recent = recent;
        self
    }
    pub(in crate::ui) fn id(&self) -> &str {
        &self.id
    }
    pub(in crate::ui) fn kind(&self) -> SearchKind {
        self.kind
    }
    pub(in crate::ui) fn title(&self) -> &str {
        &self.title
    }
    pub(in crate::ui) fn category(&self) -> &str {
        &self.category
    }
    pub(in crate::ui) fn target(&self) -> &SearchTarget {
        &self.target
    }
    pub(in crate::ui) fn recent(&self) -> Option<u64> {
        self.recent
    }
    pub(in crate::ui) fn matches(&self, query: &str) -> bool {
        query
            .to_lowercase()
            .split_whitespace()
            .all(|word| self.terms.contains(word))
    }
}
