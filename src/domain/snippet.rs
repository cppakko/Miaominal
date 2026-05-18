use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnippetRecord {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub package: String,
    #[serde(default = "default_snippet_language")]
    pub language: String,
    #[serde(default)]
    pub script: String,
}

fn default_snippet_language() -> String {
    "bash".into()
}

pub(crate) fn matches_filter(snippet: &SnippetRecord, filter_text: &str) -> bool {
    let filter_text = filter_text.trim().to_ascii_lowercase();
    if filter_text.is_empty() {
        return true;
    }

    format!(
        "{} {} {} {}",
        snippet.description, snippet.package, snippet.language, snippet.script,
    )
    .to_ascii_lowercase()
    .contains(&filter_text)
}

pub(crate) fn package_initials(package: &str) -> Option<String> {
    let mut initials = String::new();
    for token in package
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
    {
        if let Some(character) = token.chars().next() {
            initials.push(character.to_ascii_uppercase());
        }
        if initials.chars().count() >= 3 {
            break;
        }
    }

    if initials.is_empty() {
        for character in package
            .chars()
            .filter(|character| character.is_alphanumeric())
        {
            initials.push(character.to_ascii_uppercase());
            if initials.chars().count() >= 3 {
                break;
            }
        }
    }

    (!initials.is_empty()).then_some(initials)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_filter_searches_visible_snippet_fields() {
        let snippet = SnippetRecord {
            id: "snippet-1".into(),
            description: "Rotate logs".into(),
            package: "Maintenance".into(),
            language: "bash".into(),
            script: "logrotate -f /etc/logrotate.conf".into(),
        };

        assert!(matches_filter(&snippet, "rotate"));
        assert!(matches_filter(&snippet, "maintenance"));
        assert!(matches_filter(&snippet, "LOGROTATE"));
        assert!(!matches_filter(&snippet, "database"));
    }

    #[test]
    fn package_initials_prefers_token_boundaries() {
        assert_eq!(package_initials("ops-tools"), Some("OT".into()));
        assert_eq!(package_initials("database backup jobs"), Some("DBJ".into()));
        assert_eq!(package_initials("  "), None);
    }
}
