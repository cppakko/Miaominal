use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetVariable {
    pub name: String,
    pub default_value: Option<String>,
}

/// Extract unique `{{var}}` / `{{var:default}}` placeholders in order of first appearance.
pub fn extract_snippet_variables(script: &str) -> Vec<SnippetVariable> {
    let mut vars = Vec::new();
    let mut seen = HashSet::new();
    let mut rest = script;
    while let Some(start) = rest.find("{{") {
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}}") else {
            break;
        };
        let token = after_open[..end].trim();
        if !token.is_empty() && !token.contains('\n') && !token.contains("{{") {
            let (name, default_value) = match token.split_once(':') {
                Some((name, default)) => (name.trim(), Some(default.trim().to_string())),
                None => (token, None),
            };
            if !name.is_empty() && seen.insert(name.to_string()) {
                vars.push(SnippetVariable {
                    name: name.to_string(),
                    default_value,
                });
            }
        }
        rest = &after_open[end + 2..];
    }
    vars
}

/// Substitute `{{var}}` and `{{var:default}}` placeholders using `values`.
pub fn render_snippet_script(script: &str, values: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(script.len());
    let mut rest = script;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let token = after_open[..end].trim();
        if !token.is_empty() && !token.contains('\n') && !token.contains("{{") {
            let (name, default_value) = match token.split_once(':') {
                Some((name, default)) => (name.trim(), Some(default.trim())),
                None => (token, None),
            };
            if !name.is_empty() {
                if let Some(value) = values.get(name).filter(|v| !v.is_empty()) {
                    out.push_str(value);
                } else if let Some(default) = default_value {
                    out.push_str(default);
                } else {
                    out.push_str(&rest[start..start + 2 + end + 2]);
                }
                rest = &after_open[end + 2..];
                continue;
            }
        }
        out.push_str(&rest[start..start + 2 + end + 2]);
        rest = &after_open[end + 2..];
    }
    out.push_str(rest);
    out
}

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

pub fn matches_filter(snippet: &SnippetRecord, filter_text: &str) -> bool {
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

pub fn package_initials(package: &str) -> Option<String> {
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

    #[test]
    fn extract_and_render_snippet_variables_handle_defaults_and_duplicates() {
        let script = "systemctl restart {{service:nginx}} && curl -I http://127.0.0.1:{{port:8080}}/{{service}}";
        let vars = extract_snippet_variables(script);
        assert_eq!(
            vars,
            vec![
                SnippetVariable {
                    name: "service".into(),
                    default_value: Some("nginx".into()),
                },
                SnippetVariable {
                    name: "port".into(),
                    default_value: Some("8080".into()),
                },
            ]
        );

        let mut values = HashMap::new();
        values.insert("service".into(), "caddy".into());
        assert_eq!(
            render_snippet_script(script, &values),
            "systemctl restart caddy && curl -I http://127.0.0.1:8080/caddy"
        );
    }
}
