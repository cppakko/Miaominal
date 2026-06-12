use language::{
    Language, LanguageConfig, LanguageMatcher, LanguageName, LanguageQueries, LanguageRegistry,
};
use std::{borrow::Cow, sync::Arc};

const TYPESCRIPT_HIGHLIGHTS: &str = r#"
(identifier) @variable
(property_identifier) @property
(string) @string
(template_string) @string
(number) @number
(true) @boolean
(false) @boolean
(null) @constant
(comment) @comment
[
  "as"
  "async"
  "await"
  "break"
  "case"
  "catch"
  "class"
  "const"
  "continue"
  "debugger"
  "default"
  "delete"
  "do"
  "else"
  "export"
  "extends"
  "finally"
  "for"
  "from"
  "function"
  "if"
  "import"
  "in"
  "instanceof"
  "interface"
  "let"
  "new"
  "of"
  "return"
  "switch"
  "throw"
  "try"
  "type"
  "typeof"
  "var"
  "void"
  "while"
  "with"
  "yield"
] @keyword
"#;

pub fn register(registry: &Arc<LanguageRegistry>) {
    add_language(
        registry,
        "Rust",
        &["rs"],
        &["rust"],
        Some(tree_sitter_rust::LANGUAGE.into()),
        tree_sitter_rust::HIGHLIGHTS_QUERY,
    );
    add_language(
        registry,
        "Python",
        &["py", "pyw"],
        &["python", "py"],
        Some(tree_sitter_python::LANGUAGE.into()),
        tree_sitter_python::HIGHLIGHTS_QUERY,
    );
    add_language(
        registry,
        "Bash",
        &["sh", "bash", "zsh"],
        &["bash", "sh", "shell", "zsh"],
        Some(tree_sitter_bash::LANGUAGE.into()),
        tree_sitter_bash::HIGHLIGHT_QUERY,
    );
    add_language(
        registry,
        "JSON",
        &["json"],
        &["json"],
        Some(tree_sitter_json::LANGUAGE.into()),
        tree_sitter_json::HIGHLIGHTS_QUERY,
    );
    add_language(
        registry,
        "CSS",
        &["css"],
        &["css"],
        Some(tree_sitter_css::LANGUAGE.into()),
        tree_sitter_css::HIGHLIGHTS_QUERY,
    );
    add_language(
        registry,
        "TypeScript",
        &["ts", "tsx", "js", "jsx", "mjs", "cjs"],
        &["typescript", "ts", "javascript", "js", "jsx", "tsx"],
        Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        TYPESCRIPT_HIGHLIGHTS,
    );
}

fn add_language(
    registry: &Arc<LanguageRegistry>,
    name: &'static str,
    suffixes: &[&str],
    aliases: &[&str],
    parser: Option<tree_sitter::Language>,
    highlights: &'static str,
) {
    let language = Language::new(
        LanguageConfig {
            name: LanguageName::new_static(name),
            code_fence_block_name: aliases.first().map(|alias| Arc::<str>::from(*alias)),
            matcher: LanguageMatcher {
                path_suffixes: suffixes.iter().map(|suffix| suffix.to_string()).collect(),
                modeline_aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
                ..Default::default()
            },
            ..Default::default()
        },
        parser,
    )
    .with_queries(LanguageQueries {
        highlights: Some(Cow::Borrowed(highlights)),
        ..Default::default()
    });

    match language {
        Ok(language) => registry.add(Arc::new(language)),
        Err(error) => log::warn!("failed to register {name} markdown highlighting: {error:?}"),
    }
}
