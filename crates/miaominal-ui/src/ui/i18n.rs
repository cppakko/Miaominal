use miaominal_settings::AppLanguage;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

struct Catalogs {
    english: HashMap<String, String>,
    simplified_chinese: HashMap<String, String>,
}

struct I18nState {
    language: AppLanguage,
}

static CATALOGS: LazyLock<Catalogs> = LazyLock::new(|| Catalogs {
    english: load_catalog(include_str!("i18n/locales/en.toml"), "en"),
    simplified_chinese: load_catalog(include_str!("i18n/locales/zh-CN.toml"), "zh-CN"),
});

static I18N_STATE: RwLock<I18nState> = RwLock::new(I18nState {
    language: AppLanguage::English,
});

pub fn init() {
    let _ = &*CATALOGS;
    set_language(AppLanguage::detect_system());
}

pub fn current_language() -> AppLanguage {
    I18N_STATE.read().expect("i18n state poisoned").language
}

pub fn set_language(language: AppLanguage) -> bool {
    let _ = &*CATALOGS;
    let mut state = I18N_STATE.write().expect("i18n state poisoned");
    let changed = state.language != language;
    state.language = language;
    changed
}

pub fn string(key: &str) -> String {
    lookup_message(current_language(), key)
}

pub fn string_args(key: &str, arguments: &[(&str, &str)]) -> String {
    render_template(&lookup_message(current_language(), key), arguments)
}

fn lookup_message(language: AppLanguage, key: &str) -> String {
    let primary_catalog = match language {
        AppLanguage::English => &CATALOGS.english,
        AppLanguage::SimplifiedChinese => &CATALOGS.simplified_chinese,
    };

    primary_catalog
        .get(key)
        .or_else(|| CATALOGS.english.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

fn render_template(template: &str, arguments: &[(&str, &str)]) -> String {
    let mut rendered = template.to_string();
    for (name, value) in arguments {
        let placeholder = format!("{{{name}}}");
        rendered = rendered.replace(&placeholder, value);
    }
    rendered
}

fn load_catalog(contents: &str, language_code: &str) -> HashMap<String, String> {
    let parsed = match contents.parse::<toml::Value>() {
        Ok(value) => value,
        Err(error) => {
            log::error!("failed to parse {language_code} locale catalog: {error}");
            return HashMap::new();
        }
    };

    let mut catalog = HashMap::new();
    flatten_catalog(None, &parsed, &mut catalog);
    catalog
}

fn flatten_catalog(
    prefix: Option<&str>,
    value: &toml::Value,
    catalog: &mut HashMap<String, String>,
) {
    match value {
        toml::Value::Table(table) => {
            for (key, nested_value) in table {
                let nested_prefix = match prefix {
                    Some(existing_prefix) => format!("{existing_prefix}.{key}"),
                    None => key.clone(),
                };
                flatten_catalog(Some(&nested_prefix), nested_value, catalog);
            }
        }
        toml::Value::String(text) => {
            if let Some(key) = prefix {
                catalog.insert(key.to_string(), text.clone());
            }
        }
        _ => {
            if let Some(key) = prefix {
                log::warn!("ignoring non-string i18n value for key {key}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    fn append_rust_sources(dir: &Path, output: &mut String) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                append_rust_sources(&path, output)?;
                continue;
            }

            if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                output.push_str(&fs::read_to_string(&path)?);
                output.push('\n');
            }
        }

        Ok(())
    }

    #[test]
    fn locale_catalogs_have_matching_keys() {
        let english_keys: BTreeSet<_> = CATALOGS.english.keys().cloned().collect();
        let chinese_keys: BTreeSet<_> = CATALOGS.simplified_chinese.keys().cloned().collect();

        assert_eq!(english_keys, chinese_keys);
    }

    #[test]
    fn locale_catalogs_do_not_contain_unused_keys() -> std::io::Result<()> {
        let mut rust_sources = String::new();
        let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        append_rust_sources(&src_dir, &mut rust_sources)?;

        let mut unused_keys: Vec<_> = CATALOGS
            .english
            .keys()
            .filter(|key| !rust_sources.contains(&format!("\"{key}\"")))
            .cloned()
            .collect();
        unused_keys.sort();

        assert!(
            unused_keys.is_empty(),
            "unused i18n keys: {}",
            unused_keys.join(", ")
        );

        Ok(())
    }

    #[test]
    fn template_render_replaces_named_arguments() {
        let rendered = render_template("Language: {language}", &[("language", "English")]);

        assert_eq!(rendered, "Language: English");
    }
}
