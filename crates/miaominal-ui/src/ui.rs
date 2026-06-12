pub mod assets;
pub(crate) mod components;
pub mod i18n;
mod markdown_languages;
mod shell;
pub(crate) mod theme;
pub(crate) mod utils;

use ::theme::ActiveTheme as _;
use gpui::{App, FontStyle, FontWeight, Global, HighlightStyle, Hsla, KeyBinding};
use language::LanguageRegistry;
use settings::Settings as _;
pub use shell::AppView;
use std::sync::Arc;

struct MarkdownLanguageRegistry(Arc<LanguageRegistry>);

impl Global for MarkdownLanguageRegistry {}

pub fn init_zed_markdown(cx: &mut App) {
    if !cx.has_global::<settings::SettingsStore>() {
        settings::init(cx);
    }

    theme_settings::ThemeSettings::register(cx);

    if !cx.has_global::<::theme::GlobalTheme>() {
        theme_settings::init(::theme::LoadThemes::JustBase, cx);
    }
    sync_markdown_theme(cx);
    cx.bind_keys([
        KeyBinding::new("ctrl-c", zed_markdown::Copy, Some("Markdown")),
        KeyBinding::new("cmd-c", zed_markdown::Copy, Some("Markdown")),
    ]);

    if !cx.has_global::<MarkdownLanguageRegistry>() {
        let registry = Arc::new(LanguageRegistry::new(cx.background_executor().clone()));
        registry.set_theme(cx.theme().clone());
        markdown_languages::register(&registry);
        cx.set_global(MarkdownLanguageRegistry(registry));
    }
}

pub(crate) fn markdown_language_registry(cx: &mut App) -> Arc<LanguageRegistry> {
    let registry = cx.global::<MarkdownLanguageRegistry>().0.clone();
    registry.set_theme(cx.theme().clone());
    registry
}

/// Sync the zed GlobalTheme colors to match the current miaominal material theme.
/// Must be called after any miaominal theme change so the markdown renderer picks up
/// the correct text, border, and surface colors.
pub fn sync_markdown_theme(cx: &mut App) {
    let roles = miaominal_settings::current_theme().material.roles;
    let to_hsla = |color: u32| -> Hsla { gpui::rgb(color).into() };

    let mut theme = (**cx.theme()).clone();
    theme.styles.syntax = markdown_syntax_theme();
    theme.styles.colors.text = to_hsla(roles.on_surface);
    theme.styles.colors.text_muted = to_hsla(roles.on_surface_variant);
    theme.styles.colors.border = to_hsla(roles.outline);
    theme.styles.colors.border_variant = to_hsla(roles.outline_variant);
    theme.styles.colors.title_bar_background = to_hsla(roles.surface_container);
    theme.styles.colors.panel_background = to_hsla(roles.surface_container_low);
    theme.styles.colors.editor_background = to_hsla(roles.surface_container_low);
    ::theme::GlobalTheme::update_theme(cx, Arc::new(theme));
}

fn markdown_syntax_theme() -> Arc<::theme::SyntaxTheme> {
    let blue = Hsla {
        h: 207.8 / 360.0,
        s: 0.81,
        l: 0.66,
        a: 1.0,
    };
    let gray = Hsla {
        h: 218.8 / 360.0,
        s: 0.10,
        l: 0.40,
        a: 1.0,
    };
    let green = Hsla {
        h: 95.0 / 360.0,
        s: 0.38,
        l: 0.62,
        a: 1.0,
    };
    let orange = Hsla {
        h: 29.0 / 360.0,
        s: 0.54,
        l: 0.61,
        a: 1.0,
    };
    let purple = Hsla {
        h: 286.0 / 360.0,
        s: 0.51,
        l: 0.64,
        a: 1.0,
    };
    let red = Hsla {
        h: 355.0 / 360.0,
        s: 0.65,
        l: 0.65,
        a: 1.0,
    };
    let teal = Hsla {
        h: 187.0 / 360.0,
        s: 0.47,
        l: 0.55,
        a: 1.0,
    };
    let yellow = Hsla {
        h: 39.0 / 360.0,
        s: 0.67,
        l: 0.69,
        a: 1.0,
    };

    Arc::new(::theme::SyntaxTheme::new(vec![
        ("attribute".into(), purple.into()),
        ("boolean".into(), orange.into()),
        ("comment".into(), gray.into()),
        ("comment.doc".into(), gray.into()),
        ("constant".into(), yellow.into()),
        ("constructor".into(), blue.into()),
        ("embedded".into(), HighlightStyle::default()),
        (
            "emphasis".into(),
            HighlightStyle {
                font_style: Some(FontStyle::Italic),
                ..HighlightStyle::default()
            },
        ),
        (
            "emphasis.strong".into(),
            HighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                ..HighlightStyle::default()
            },
        ),
        ("enum".into(), teal.into()),
        ("function".into(), blue.into()),
        ("function.method".into(), blue.into()),
        ("function.definition".into(), blue.into()),
        ("hint".into(), blue.into()),
        ("keyword".into(), purple.into()),
        ("label".into(), HighlightStyle::default()),
        ("link_text".into(), blue.into()),
        (
            "link_uri".into(),
            HighlightStyle {
                color: Some(teal),
                font_style: Some(FontStyle::Italic),
                ..HighlightStyle::default()
            },
        ),
        ("number".into(), orange.into()),
        ("operator".into(), yellow.into()),
        ("predictive".into(), HighlightStyle::default()),
        ("preproc".into(), purple.into()),
        ("primary".into(), HighlightStyle::default()),
        ("property".into(), red.into()),
        ("punctuation".into(), HighlightStyle::default()),
        ("punctuation.bracket".into(), HighlightStyle::default()),
        ("punctuation.delimiter".into(), HighlightStyle::default()),
        ("punctuation.list_marker".into(), HighlightStyle::default()),
        ("punctuation.special".into(), HighlightStyle::default()),
        ("string".into(), green.into()),
        ("string.escape".into(), yellow.into()),
        ("string.regex".into(), red.into()),
        ("string.special".into(), green.into()),
        ("string.special.symbol".into(), green.into()),
        ("tag".into(), blue.into()),
        ("text.literal".into(), green.into()),
        ("title".into(), blue.into()),
        ("type".into(), teal.into()),
        ("variable".into(), HighlightStyle::default()),
        ("variable.special".into(), red.into()),
        ("variant".into(), HighlightStyle::default()),
        ("diff.plus".into(), green.into()),
        ("diff.minus".into(), red.into()),
    ]))
}
