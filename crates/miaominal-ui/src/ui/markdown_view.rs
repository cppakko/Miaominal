use gpui::{
    AnyElement, App, Background, Bounds, ClipboardItem, DispatchPhase, Element, ElementId,
    FontStyle, FontWeight, GlobalElementId, HighlightStyle, Hitbox, HitboxBehavior, Hsla,
    InspectorElementId, IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, SharedString, Styled, StyledText, TextAlign, Window, div, fill, px,
};
use gpui_component::{scroll::ScrollableElement, v_flex};
use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, MetadataBlockKind, Options,
    Parser, Tag, TagEnd,
};
use std::collections::{HashMap, VecDeque, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    FontStyle as SyntectFontStyle, Style as SyntectStyle, Theme, ThemeSet,
};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

use crate::ui::{assets::AppIcon, components::icon_button};

const HIGHLIGHT_MAX_BYTES: usize = 32 * 1024;
const MARKDOWN_CACHE_MAX_ENTRIES: usize = 96;
const MARKDOWN_CACHE_MAX_SOURCE_BYTES: usize = 512 * 1024;
const CODE_HIGHLIGHT_CACHE_MAX_ENTRIES: usize = 256;

#[derive(Clone, Copy)]
pub(crate) struct MarkdownViewStyle {
    pub(crate) text_color: Hsla,
    pub(crate) muted_color: Hsla,
    pub(crate) link_color: Hsla,
    pub(crate) code_background: Hsla,
    pub(crate) border_color: Hsla,
}

pub(crate) fn render_markdown_selectable(
    source: &str,
    style: MarkdownViewStyle,
    selection: Option<&MarkdownTextSelection>,
    selection_handlers: Option<&MarkdownTextSelectionHandlers>,
) -> AnyElement {
    let document = cached_markdown_document(source, style);
    document.render(MarkdownRenderContext {
        style,
        selection,
        selection_handlers,
    })
}

pub(crate) fn render_markdown_uncached_selectable(
    source: &str,
    style: MarkdownViewStyle,
    selection: Option<&MarkdownTextSelection>,
    selection_handlers: Option<&MarkdownTextSelectionHandlers>,
) -> AnyElement {
    let mut renderer = MarkdownRenderer::new(style);
    renderer.parse(source).render(MarkdownRenderContext {
        style,
        selection,
        selection_handlers,
    })
}

pub(crate) fn markdown_plain_blocks(
    source: &str,
    style: MarkdownViewStyle,
) -> Vec<(usize, String)> {
    cached_markdown_document(source, style).plain_blocks()
}

pub(crate) struct MarkdownTextSelectionHandlers {
    pub(crate) on_start: Rc<dyn Fn(usize, usize, &mut Window, &mut App)>,
    pub(crate) on_update: Rc<dyn Fn(usize, usize, &mut Window, &mut App)>,
    pub(crate) on_finish: Rc<dyn Fn(usize, usize, &mut Window, &mut App)>,
}

pub(crate) fn code_highlights_for_language(
    language: &str,
    code: &str,
) -> Vec<(Range<usize>, HighlightStyle)> {
    cached_syntect_highlights(language, code).unwrap_or_default()
}

struct MarkdownRenderer {
    style: MarkdownViewStyle,
    blocks: Vec<MarkdownBlock>,
    inline: InlineBuffer,
    stack: Vec<InlineStyle>,
    list_stack: Vec<ListState>,
    block_stack: Vec<BlockContext>,
    quote_depth: usize,
    code_block: Option<CodeBlock>,
    html_block: Option<String>,
    metadata_block: Option<MetadataBlockKind>,
    table: Option<TableState>,
    current_footnote: Option<String>,
    current_definition: Option<DefinitionPart>,
}

#[derive(Clone)]
struct MarkdownDocument {
    blocks: Vec<MarkdownBlock>,
}

struct MarkdownRenderContext<'a> {
    style: MarkdownViewStyle,
    selection: Option<&'a MarkdownTextSelection>,
    selection_handlers: Option<&'a MarkdownTextSelectionHandlers>,
}

pub(crate) struct MarkdownTextSelection {
    pub(crate) start_block: usize,
    pub(crate) start_offset: usize,
    pub(crate) end_block: usize,
    pub(crate) end_offset: usize,
    pub(crate) color: Hsla,
}

#[derive(Clone)]
enum MarkdownBlock {
    Paragraph(InlineSnapshot),
    Heading {
        level: HeadingLevel,
        content: InlineSnapshot,
    },
    ListItem {
        marker: ListMarker,
        depth: usize,
        content: InlineSnapshot,
    },
    Quote {
        kind: Option<BlockQuoteKind>,
        depth: usize,
        content: InlineSnapshot,
    },
    FootnoteDefinition {
        label: String,
        content: InlineSnapshot,
    },
    Definition {
        is_title: bool,
        content: InlineSnapshot,
    },
    Code {
        language: String,
        text: String,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
    },
    Special {
        label: String,
        content: InlineSnapshot,
    },
    Rule,
    Table {
        alignments: Vec<Alignment>,
        rows: Vec<TableRow>,
    },
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct MarkdownCacheKey {
    source_hash: u64,
    source_len: usize,
    style: MarkdownStyleKey,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct MarkdownStyleKey {
    text_color: ColorKey,
    muted_color: ColorKey,
    link_color: ColorKey,
    code_background: ColorKey,
    border_color: ColorKey,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct ColorKey {
    h: u32,
    s: u32,
    l: u32,
    a: u32,
}

#[derive(Default)]
struct MarkdownDocumentCache {
    entries: HashMap<MarkdownCacheKey, MarkdownDocument>,
    order: VecDeque<MarkdownCacheKey>,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct CodeHighlightCacheKey {
    language_hash: u64,
    language_len: usize,
    code_hash: u64,
    code_len: usize,
}

#[derive(Default)]
struct CodeHighlightCache {
    entries: HashMap<CodeHighlightCacheKey, Vec<(Range<usize>, HighlightStyle)>>,
    order: VecDeque<CodeHighlightCacheKey>,
}

#[derive(Default)]
struct InlineBuffer {
    text: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}

#[derive(Clone)]
struct InlineSnapshot {
    text: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}

#[derive(Clone)]
struct InlineStyle {
    start: usize,
    kind: InlineKind,
}

#[derive(Clone)]
enum InlineKind {
    Emphasis,
    Strong,
    Strikethrough,
    Link(String),
    Superscript,
    Subscript,
    Image(String),
}

struct ListState {
    kind: ListKind,
    depth: usize,
}

enum ListKind {
    Bullet,
    Ordered(u64),
}

enum BlockContext {
    Paragraph,
    Heading(HeadingLevel),
    ListItem { marker: ListMarker, depth: usize },
    BlockQuote(Option<BlockQuoteKind>),
    FootnoteDefinition(String),
    DefinitionTitle,
    DefinitionBody,
}

#[derive(Clone)]
enum ListMarker {
    Bullet,
    Ordered(u64),
    Task(bool),
}

struct CodeBlock {
    language: Option<String>,
    text: String,
}

struct TableState {
    alignments: Vec<Alignment>,
    rows: Vec<TableRow>,
    current_row: Option<TableRow>,
    in_header: bool,
}

#[derive(Clone)]
struct TableRow {
    cells: Vec<InlineSnapshot>,
    is_header: bool,
}

#[derive(Clone, Copy)]
enum DefinitionPart {
    Title,
    Body,
}

impl MarkdownRenderer {
    fn new(style: MarkdownViewStyle) -> Self {
        Self {
            style,
            blocks: Vec::new(),
            inline: InlineBuffer::default(),
            stack: Vec::new(),
            list_stack: Vec::new(),
            block_stack: Vec::new(),
            quote_depth: 0,
            code_block: None,
            html_block: None,
            metadata_block: None,
            table: None,
            current_footnote: None,
            current_definition: None,
        }
    }

    fn parse(&mut self, source: &str) -> MarkdownDocument {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_FOOTNOTES);
        options.insert(Options::ENABLE_OLD_FOOTNOTES);
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TASKLISTS);
        options.insert(Options::ENABLE_DEFINITION_LIST);
        options.insert(Options::ENABLE_SUPERSCRIPT);
        options.insert(Options::ENABLE_SUBSCRIPT);
        options.insert(Options::ENABLE_MATH);
        options.insert(Options::ENABLE_GFM);
        options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
        options.insert(Options::ENABLE_SMART_PUNCTUATION);

        for event in Parser::new_ext(source, options) {
            self.handle_event(event);
        }
        self.flush_inline_block();

        MarkdownDocument {
            blocks: std::mem::take(&mut self.blocks),
        }
    }

    fn handle_event(&mut self, event: Event<'_>) {
        if self.handle_code_block_event(&event)
            || self.handle_html_block_event(&event)
            || self.handle_metadata_block_event(&event)
        {
            return;
        }

        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.push_text(&text),
            Event::Code(code) => self.push_inline_code(&code),
            Event::InlineMath(math) => self.push_inline_math(&math),
            Event::DisplayMath(math) => {
                self.flush_inline_block();
                self.push_special_block("math", math.as_ref());
            }
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => self.push_text("\n"),
            Event::Rule => {
                self.flush_inline_block();
                self.blocks.push(MarkdownBlock::Rule);
            }
            Event::TaskListMarker(checked) => self.set_current_task_marker(checked),
            Event::Html(html) | Event::InlineHtml(html) => self.push_inline_html(&html),
            Event::FootnoteReference(label) => self.push_footnote_reference(&label),
        }
    }

    fn handle_code_block_event(&mut self, event: &Event<'_>) -> bool {
        let Some(code_block) = self.code_block.as_mut() else {
            return false;
        };

        match event {
            Event::End(TagEnd::CodeBlock) => {
                let code_block = self.code_block.take().expect("code block exists");
                self.push_code_block(code_block);
            }
            Event::Text(text)
            | Event::Code(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => code_block.text.push_str(text),
            Event::SoftBreak | Event::HardBreak => code_block.text.push('\n'),
            _ => {}
        }
        true
    }

    fn handle_html_block_event(&mut self, event: &Event<'_>) -> bool {
        let Some(html) = self.html_block.as_mut() else {
            return false;
        };

        match event {
            Event::End(TagEnd::HtmlBlock) => {
                let html = self.html_block.take().expect("html block exists");
                self.push_special_block("html", html.trim_end());
            }
            Event::Html(text) | Event::InlineHtml(text) | Event::Text(text) => {
                html.push_str(text);
            }
            Event::SoftBreak | Event::HardBreak => html.push('\n'),
            _ => {}
        }
        true
    }

    fn handle_metadata_block_event(&mut self, event: &Event<'_>) -> bool {
        let Some(kind) = self.metadata_block else {
            return false;
        };

        match event {
            Event::End(TagEnd::MetadataBlock(_)) => {
                self.metadata_block = None;
                let label = match kind {
                    MetadataBlockKind::YamlStyle => "metadata",
                    MetadataBlockKind::PlusesStyle => "metadata",
                };
                let text = self.take_inline();
                if !text.text.trim().is_empty() {
                    self.push_special_snapshot(label, text);
                }
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                self.push_text(text);
            }
            Event::SoftBreak | Event::HardBreak => self.push_text("\n"),
            _ => {}
        }
        true
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.block_stack.push(BlockContext::Paragraph),
            Tag::Heading { level, .. } => {
                self.flush_inline_block();
                self.block_stack.push(BlockContext::Heading(level));
            }
            Tag::BlockQuote(kind) => {
                self.flush_inline_block();
                self.quote_depth += 1;
                self.block_stack.push(BlockContext::BlockQuote(kind));
            }
            Tag::CodeBlock(kind) => {
                self.flush_inline_block();
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().map(str::to_string)
                    }
                    CodeBlockKind::Indented => None,
                };
                self.code_block = Some(CodeBlock {
                    language,
                    text: String::new(),
                });
            }
            Tag::HtmlBlock => {
                self.flush_inline_block();
                self.html_block = Some(String::new());
            }
            Tag::MetadataBlock(kind) => {
                self.flush_inline_block();
                self.metadata_block = Some(kind);
            }
            Tag::List(start) => {
                self.flush_inline_block();
                let depth = self.list_stack.len();
                self.list_stack.push(ListState {
                    kind: match start {
                        Some(start) => ListKind::Ordered(start),
                        None => ListKind::Bullet,
                    },
                    depth,
                });
            }
            Tag::Item => {
                self.flush_inline_block();
                let (marker, depth) = match self.list_stack.last_mut() {
                    Some(ListState {
                        kind: ListKind::Ordered(next),
                        depth,
                    }) => {
                        let marker = ListMarker::Ordered(*next);
                        *next = next.saturating_add(1);
                        (marker, *depth)
                    }
                    Some(ListState { depth, .. }) => (ListMarker::Bullet, *depth),
                    None => (ListMarker::Bullet, 0),
                };
                self.block_stack
                    .push(BlockContext::ListItem { marker, depth });
            }
            Tag::FootnoteDefinition(label) => {
                self.flush_inline_block();
                let label = label.to_string();
                self.current_footnote = Some(label.clone());
                self.block_stack
                    .push(BlockContext::FootnoteDefinition(label));
            }
            Tag::DefinitionList => {
                self.flush_inline_block();
            }
            Tag::DefinitionListTitle => {
                self.flush_inline_block();
                self.current_definition = Some(DefinitionPart::Title);
                self.block_stack.push(BlockContext::DefinitionTitle);
            }
            Tag::DefinitionListDefinition => {
                self.flush_inline_block();
                self.current_definition = Some(DefinitionPart::Body);
                self.block_stack.push(BlockContext::DefinitionBody);
            }
            Tag::Table(alignments) => {
                self.flush_inline_block();
                self.table = Some(TableState {
                    alignments,
                    rows: Vec::new(),
                    current_row: None,
                    in_header: false,
                });
            }
            Tag::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.in_header = true;
                }
            }
            Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.current_row = Some(TableRow {
                        cells: Vec::new(),
                        is_header: table.in_header,
                    });
                }
            }
            Tag::TableCell => {
                self.flush_open_inline_styles();
                self.inline = InlineBuffer::default();
            }
            Tag::Emphasis => self.push_style(InlineKind::Emphasis),
            Tag::Strong => self.push_style(InlineKind::Strong),
            Tag::Strikethrough => self.push_style(InlineKind::Strikethrough),
            Tag::Superscript => self.push_style(InlineKind::Superscript),
            Tag::Subscript => self.push_style(InlineKind::Subscript),
            Tag::Link { dest_url, .. } => self.push_style(InlineKind::Link(dest_url.to_string())),
            Tag::Image { dest_url, .. } => {
                self.push_text("Image: ");
                self.push_style(InlineKind::Image(dest_url.to_string()));
            }
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.pop_block(|block| matches!(block, BlockContext::Paragraph));
                self.flush_inline_block();
            }
            TagEnd::Heading(_) => {
                self.flush_inline_block();
                self.pop_block(|block| matches!(block, BlockContext::Heading(_)));
            }
            TagEnd::BlockQuote(_) => {
                self.flush_inline_block();
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.pop_block(|block| matches!(block, BlockContext::BlockQuote(_)));
            }
            TagEnd::List(_) => {
                self.flush_inline_block();
                self.list_stack.pop();
            }
            TagEnd::Item => {
                self.flush_inline_block();
                self.pop_block(|block| matches!(block, BlockContext::ListItem { .. }));
            }
            TagEnd::FootnoteDefinition => {
                self.flush_inline_block();
                self.current_footnote = None;
                self.pop_block(|block| matches!(block, BlockContext::FootnoteDefinition(_)));
            }
            TagEnd::DefinitionList => {
                self.flush_inline_block();
            }
            TagEnd::DefinitionListTitle => {
                self.flush_inline_block();
                self.current_definition = None;
                self.pop_block(|block| matches!(block, BlockContext::DefinitionTitle));
            }
            TagEnd::DefinitionListDefinition => {
                self.flush_inline_block();
                self.current_definition = None;
                self.pop_block(|block| matches!(block, BlockContext::DefinitionBody));
            }
            TagEnd::Table => {
                self.flush_inline_block();
                if let Some(table) = self.table.take() {
                    self.push_table(table);
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.in_header = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut()
                    && let Some(row) = table.current_row.take()
                {
                    table.rows.push(row);
                }
            }
            TagEnd::TableCell => {
                self.flush_open_inline_styles();
                let cell = self.take_inline();
                if let Some(table) = self.table.as_mut()
                    && let Some(row) = table.current_row.as_mut()
                {
                    row.cells.push(cell);
                }
            }
            TagEnd::Emphasis => self.pop_style(|kind| matches!(kind, InlineKind::Emphasis)),
            TagEnd::Strong => self.pop_style(|kind| matches!(kind, InlineKind::Strong)),
            TagEnd::Strikethrough => {
                self.pop_style(|kind| matches!(kind, InlineKind::Strikethrough))
            }
            TagEnd::Superscript => self.pop_style(|kind| matches!(kind, InlineKind::Superscript)),
            TagEnd::Subscript => self.pop_style(|kind| matches!(kind, InlineKind::Subscript)),
            TagEnd::Link => self.pop_style(|kind| matches!(kind, InlineKind::Link(_))),
            TagEnd::Image => self.pop_style(|kind| matches!(kind, InlineKind::Image(_))),
            TagEnd::CodeBlock | TagEnd::HtmlBlock | TagEnd::MetadataBlock(_) => {}
        }
    }

    fn push_style(&mut self, kind: InlineKind) {
        self.stack.push(InlineStyle {
            start: self.inline.text.len(),
            kind,
        });
    }

    fn pop_style(&mut self, predicate: impl Fn(&InlineKind) -> bool) {
        let Some(index) = self.stack.iter().rposition(|style| predicate(&style.kind)) else {
            return;
        };
        let style = self.stack.remove(index);
        self.apply_inline_style(style, self.inline.text.len());
    }

    fn apply_inline_style(&mut self, style: InlineStyle, end: usize) {
        if style.start >= end {
            return;
        }

        if let InlineKind::Link(dest) | InlineKind::Image(dest) = &style.kind
            && !dest.is_empty()
            && !self.inline.text[style.start..end].contains(dest)
        {
            let suffix = format!(" ({dest})");
            let suffix_start = self.inline.text.len();
            self.inline.text.push_str(&suffix);
            self.inline.highlights.push((
                suffix_start..self.inline.text.len(),
                HighlightStyle {
                    color: Some(self.style.muted_color),
                    ..Default::default()
                },
            ));
        }

        self.inline.highlights.push((
            style.start..end,
            highlight_for_style(self.style, &style.kind),
        ));
    }

    fn flush_open_inline_styles(&mut self) {
        let end = self.inline.text.len();
        let styles = self.stack.drain(..).collect::<Vec<_>>();
        for style in styles {
            self.apply_inline_style(style, end);
        }
    }

    fn push_text(&mut self, text: &str) {
        self.inline.text.push_str(text);
    }

    fn push_inline_code(&mut self, code: &str) {
        let start = self.inline.text.len();
        self.inline.text.push_str(code);
        self.inline.highlights.push((
            start..self.inline.text.len(),
            HighlightStyle {
                color: Some(self.style.text_color),
                background_color: Some(self.style.code_background),
                ..Default::default()
            },
        ));
    }

    fn push_inline_math(&mut self, math: &str) {
        let start = self.inline.text.len();
        self.inline.text.push('$');
        self.inline.text.push_str(math);
        self.inline.text.push('$');
        self.inline.highlights.push((
            start..self.inline.text.len(),
            HighlightStyle {
                color: Some(self.style.link_color),
                background_color: Some(self.style.code_background.opacity(0.55)),
                ..Default::default()
            },
        ));
    }

    fn push_inline_html(&mut self, html: &str) {
        let start = self.inline.text.len();
        self.inline.text.push_str(html);
        self.inline.highlights.push((
            start..self.inline.text.len(),
            HighlightStyle {
                color: Some(self.style.muted_color),
                ..Default::default()
            },
        ));
    }

    fn push_footnote_reference(&mut self, label: &str) {
        let start = self.inline.text.len();
        self.inline.text.push_str("[^");
        self.inline.text.push_str(label);
        self.inline.text.push(']');
        self.inline.highlights.push((
            start..self.inline.text.len(),
            HighlightStyle {
                color: Some(self.style.link_color),
                font_weight: Some(FontWeight::SEMIBOLD),
                ..Default::default()
            },
        ));
    }

    fn set_current_task_marker(&mut self, checked: bool) {
        if let Some(BlockContext::ListItem { marker, .. }) = self
            .block_stack
            .iter_mut()
            .rev()
            .find(|block| matches!(block, BlockContext::ListItem { .. }))
        {
            *marker = ListMarker::Task(checked);
        }
    }

    fn flush_inline_block(&mut self) {
        self.flush_open_inline_styles();
        let snapshot = self.take_inline();
        if snapshot.text.trim().is_empty() {
            return;
        }

        if self.table.is_some() {
            self.inline = InlineBuffer {
                text: snapshot.text,
                highlights: snapshot.highlights,
            };
            return;
        }

        let block = self.current_block_context();
        let markdown_block = match block {
            Some(BlockContext::Heading(level)) => MarkdownBlock::Heading {
                level: *level,
                content: snapshot,
            },
            Some(BlockContext::ListItem { marker, depth }) => MarkdownBlock::ListItem {
                marker: marker.clone(),
                depth: *depth,
                content: snapshot,
            },
            Some(BlockContext::BlockQuote(kind)) => MarkdownBlock::Quote {
                kind: *kind,
                depth: self.quote_depth,
                content: snapshot,
            },
            Some(BlockContext::FootnoteDefinition(label)) => MarkdownBlock::FootnoteDefinition {
                label: label.clone(),
                content: snapshot,
            },
            Some(BlockContext::DefinitionTitle) => MarkdownBlock::Definition {
                is_title: true,
                content: snapshot,
            },
            Some(BlockContext::DefinitionBody) => MarkdownBlock::Definition {
                is_title: false,
                content: snapshot,
            },
            _ => match self.current_definition {
                Some(DefinitionPart::Title) => MarkdownBlock::Definition {
                    is_title: true,
                    content: snapshot,
                },
                Some(DefinitionPart::Body) => MarkdownBlock::Definition {
                    is_title: false,
                    content: snapshot,
                },
                None => MarkdownBlock::Paragraph(snapshot),
            },
        };

        self.blocks.push(markdown_block);
    }

    fn take_inline(&mut self) -> InlineSnapshot {
        let text = std::mem::take(&mut self.inline.text);
        let highlights = valid_highlights(&text, std::mem::take(&mut self.inline.highlights));
        InlineSnapshot { text, highlights }
    }

    fn current_block_context(&self) -> Option<&BlockContext> {
        self.block_stack.iter().rev().find(|block| {
            matches!(
                block,
                BlockContext::Heading(_)
                    | BlockContext::ListItem { .. }
                    | BlockContext::BlockQuote(_)
                    | BlockContext::FootnoteDefinition(_)
                    | BlockContext::DefinitionTitle
                    | BlockContext::DefinitionBody
            )
        })
    }

    fn pop_block(&mut self, predicate: impl Fn(&BlockContext) -> bool) {
        if let Some(index) = self.block_stack.iter().rposition(predicate) {
            self.block_stack.remove(index);
        }
    }

    fn push_code_block(&mut self, code_block: CodeBlock) {
        let text = code_block.text.trim_end_matches('\n').to_string();
        let language = code_block.language.unwrap_or_default();
        let highlights = (!language.is_empty())
            .then(|| cached_syntect_highlights(&language, &text))
            .flatten();
        self.blocks.push(MarkdownBlock::Code {
            language,
            text,
            highlights: highlights.unwrap_or_default(),
        });
    }

    fn push_special_block(&mut self, label: &str, text: &str) {
        if text.trim().is_empty() {
            return;
        }

        self.push_special_snapshot(
            label,
            InlineSnapshot {
                text: text.to_string(),
                highlights: Vec::new(),
            },
        );
    }

    fn push_special_snapshot(&mut self, label: &str, snapshot: InlineSnapshot) {
        self.blocks.push(MarkdownBlock::Special {
            label: label.to_string(),
            content: snapshot,
        });
    }

    fn push_table(&mut self, table: TableState) {
        if table.rows.is_empty() {
            return;
        }

        let column_count = table.alignments.len().max(
            table
                .rows
                .iter()
                .map(|row| row.cells.len())
                .max()
                .unwrap_or(0),
        );
        if column_count == 0 {
            return;
        }

        self.blocks.push(MarkdownBlock::Table {
            alignments: table.alignments,
            rows: table.rows,
        });
    }
}

impl MarkdownDocumentCache {
    fn get(&self, key: &MarkdownCacheKey) -> Option<MarkdownDocument> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: MarkdownCacheKey, document: MarkdownDocument) {
        if self.entries.insert(key, document).is_none() {
            self.order.push_back(key);
        }
        self.evict_old_entries(MARKDOWN_CACHE_MAX_ENTRIES);
    }

    fn evict_old_entries(&mut self, max_entries: usize) {
        while self.entries.len() > max_entries {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&key);
        }
    }
}

impl CodeHighlightCache {
    fn get(&self, key: &CodeHighlightCacheKey) -> Option<Vec<(Range<usize>, HighlightStyle)>> {
        self.entries.get(key).cloned()
    }

    fn insert(
        &mut self,
        key: CodeHighlightCacheKey,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
    ) {
        if self.entries.insert(key, highlights).is_none() {
            self.order.push_back(key);
        }
        self.evict_old_entries(CODE_HIGHLIGHT_CACHE_MAX_ENTRIES);
    }

    fn evict_old_entries(&mut self, max_entries: usize) {
        while self.entries.len() > max_entries {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&key);
        }
    }
}

impl MarkdownDocument {
    fn render(&self, context: MarkdownRenderContext<'_>) -> AnyElement {
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1p5()
            .children(
                self.blocks
                    .iter()
                    .enumerate()
                    .map(|(index, block)| render_markdown_block(block, index, &context)),
            )
            .into_any_element()
    }

    fn plain_blocks(&self) -> Vec<(usize, String)> {
        let mut blocks = Vec::new();
        for (index, block) in self.blocks.iter().enumerate() {
            match block {
                MarkdownBlock::Paragraph(snapshot)
                | MarkdownBlock::Heading {
                    content: snapshot, ..
                }
                | MarkdownBlock::ListItem {
                    content: snapshot, ..
                }
                | MarkdownBlock::Quote {
                    content: snapshot, ..
                }
                | MarkdownBlock::FootnoteDefinition {
                    content: snapshot, ..
                }
                | MarkdownBlock::Definition {
                    content: snapshot, ..
                }
                | MarkdownBlock::Special {
                    content: snapshot, ..
                } => blocks.push((index, snapshot.text.clone())),
                MarkdownBlock::Code { text, .. } => blocks.push((index, text.clone())),
                MarkdownBlock::Table { rows, .. } => {
                    for (row_index, row) in rows.iter().enumerate() {
                        for (column, cell) in row.cells.iter().enumerate() {
                            blocks.push((
                                index
                                    .saturating_mul(10_000)
                                    .saturating_add(row_index.saturating_mul(100))
                                    .saturating_add(column),
                                cell.text.clone(),
                            ));
                        }
                    }
                }
                MarkdownBlock::Rule => {}
            }
        }
        blocks
    }
}

fn render_markdown_block(
    block: &MarkdownBlock,
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    let style = context.style;
    match block {
        MarkdownBlock::Paragraph(snapshot) => {
            render_paragraph(snapshot.clone(), block_index, context)
        }
        MarkdownBlock::Heading { level, content } => {
            render_heading(content.clone(), *level, block_index, context)
        }
        MarkdownBlock::ListItem {
            marker,
            depth,
            content,
        } => render_list_item(content.clone(), marker, *depth, block_index, context),
        MarkdownBlock::Quote {
            kind,
            depth,
            content,
        } => render_quote(content.clone(), *kind, *depth, block_index, context),
        MarkdownBlock::FootnoteDefinition { label, content } => {
            render_footnote_definition(content.clone(), label, block_index, context)
        }
        MarkdownBlock::Definition { is_title, content } => {
            render_definition(content.clone(), *is_title, block_index, context)
        }
        MarkdownBlock::Code {
            language,
            text,
            highlights,
        } => render_code_block(language, text, highlights.clone(), block_index, context),
        MarkdownBlock::Special { label, content } => {
            render_special_block(label, content.clone(), block_index, context)
        }
        MarkdownBlock::Rule => div()
            .w_full()
            .h(px(1.0))
            .my_1p5()
            .bg(style.border_color)
            .into_any_element(),
        MarkdownBlock::Table { alignments, rows } => {
            render_table(alignments, rows, block_index, context)
        }
    }
}

fn render_paragraph(
    snapshot: InlineSnapshot,
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    div()
        .w_full()
        .min_w_0()
        .text_size(miaominal_settings::FontSize::Input.scaled())
        .line_height(miaominal_settings::scaled_line_height(21.0))
        .text_color(context.style.text_color)
        .child(selectable_styled_text(snapshot, block_index, context))
        .into_any_element()
}

fn render_heading(
    snapshot: InlineSnapshot,
    level: HeadingLevel,
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    let style = context.style;
    let mut block = div()
        .w_full()
        .min_w_0()
        .mt(match level {
            HeadingLevel::H1 => px(10.0),
            HeadingLevel::H2 => px(8.0),
            HeadingLevel::H3 => px(6.0),
            _ => px(4.0),
        })
        .mb(match level {
            HeadingLevel::H1 | HeadingLevel::H2 => px(4.0),
            _ => px(2.0),
        })
        .text_size(match level {
            HeadingLevel::H1 => px(22.0),
            HeadingLevel::H2 => px(19.0),
            HeadingLevel::H3 => px(17.0),
            HeadingLevel::H4 => px(15.0),
            _ => px(14.0),
        })
        .line_height(match level {
            HeadingLevel::H1 => px(29.0),
            HeadingLevel::H2 => px(26.0),
            HeadingLevel::H3 => px(23.0),
            _ => miaominal_settings::scaled_line_height(21.0),
        })
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(style.text_color)
        .child(selectable_styled_text(snapshot, block_index, context));

    if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
        block = block.pb_1().border_b_1().border_color(style.border_color);
    }
    block.into_any_element()
}

fn render_list_item(
    snapshot: InlineSnapshot,
    marker: &ListMarker,
    depth: usize,
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    let style = context.style;
    let marker_element = match marker {
        ListMarker::Task(checked) => task_marker(*checked, style),
        _ => div()
            .w(px(22.0))
            .flex_none()
            .pt(px(1.0))
            .text_color(style.muted_color)
            .text_size(miaominal_settings::FontSize::Input.scaled())
            .line_height(miaominal_settings::scaled_line_height(21.0))
            .child(match marker {
                ListMarker::Bullet => "-".to_string(),
                ListMarker::Ordered(number) => format!("{number}."),
                ListMarker::Task(_) => String::new(),
            })
            .into_any_element(),
    };

    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_start()
        .gap_1()
        .ml(px((depth as f32) * 18.0))
        .text_size(miaominal_settings::FontSize::Input.scaled())
        .line_height(miaominal_settings::scaled_line_height(21.0))
        .text_color(style.text_color)
        .child(marker_element)
        .child(div().flex_1().min_w_0().child(selectable_styled_text(
            snapshot,
            block_index,
            context,
        )))
        .into_any_element()
}

fn render_quote(
    snapshot: InlineSnapshot,
    kind: Option<BlockQuoteKind>,
    depth: usize,
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    let style = context.style;
    let label = kind.map(blockquote_kind_label);
    let content = div()
        .flex_1()
        .min_w_0()
        .text_size(miaominal_settings::FontSize::Input.scaled())
        .line_height(miaominal_settings::scaled_line_height(21.0))
        .text_color(style.muted_color)
        .child(selectable_styled_text(snapshot, block_index, context));

    let mut row = div()
        .w_full()
        .min_w_0()
        .my_0p5()
        .pl(px(10.0 + (depth.saturating_sub(1) as f32 * 8.0)))
        .pr_2()
        .py_1()
        .border_l_2()
        .border_color(style.border_color)
        .bg(style.code_background.opacity(0.28))
        .flex()
        .flex_col()
        .gap_0p5();

    if let Some(label) = label {
        row = row.child(
            div()
                .text_size(px(11.0))
                .line_height(px(14.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(style.link_color)
                .child(label),
        );
    }

    row.child(content).into_any_element()
}

fn render_footnote_definition(
    snapshot: InlineSnapshot,
    label: &str,
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    let style = context.style;
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_start()
        .gap_2()
        .mt_1()
        .text_size(px(12.0))
        .line_height(px(18.0))
        .text_color(style.muted_color)
        .child(
            div()
                .flex_none()
                .text_color(style.link_color)
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("[^{label}]")),
        )
        .child(div().flex_1().min_w_0().child(selectable_styled_text(
            snapshot,
            block_index,
            context,
        )))
        .into_any_element()
}

fn render_definition(
    snapshot: InlineSnapshot,
    is_title: bool,
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    let style = context.style;
    let mut block = div()
        .w_full()
        .min_w_0()
        .text_size(miaominal_settings::FontSize::Input.scaled())
        .line_height(miaominal_settings::scaled_line_height(21.0))
        .text_color(style.text_color);

    if is_title {
        block = block.mt_1().font_weight(FontWeight::SEMIBOLD);
    } else {
        block = block
            .ml_4()
            .pl_2()
            .border_l_1()
            .border_color(style.border_color)
            .text_color(style.muted_color);
    }

    block
        .child(selectable_styled_text(snapshot, block_index, context))
        .into_any_element()
}

fn render_code_block(
    language: &str,
    text: &str,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    let style = context.style;
    let roles = miaominal_settings::current_theme().material.roles;
    let copy_text = text.to_string();
    let language_label = if language.trim().is_empty() {
        "code".to_string()
    } else {
        language.to_string()
    };
    let snapshot = InlineSnapshot {
        text: text.to_string(),
        highlights,
    };

    div()
        .w_full()
        .min_w_0()
        .my_1()
        .rounded(px(8.0))
        .border_1()
        .border_color(style.border_color)
        .bg(style.code_background)
        .overflow_x_scrollbar()
        .font_family("JetBrains Mono")
        .text_size(miaominal_settings::FontSize::Body.scaled())
        .line_height(miaominal_settings::scaled_line_height(18.0))
        .text_color(style.text_color)
        .child(
            div()
                .w_full()
                .min_w_0()
                .px_2()
                .pt_1()
                .pb_0p5()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.0))
                        .line_height(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(style.muted_color)
                        .child(language_label),
                )
                .child(icon_button(
                    AppIcon::Copy,
                    22.0,
                    6.0,
                    Some(roles.surface_container_highest),
                    Some(roles.on_surface),
                    None,
                    move |_window, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                    },
                )),
        )
        .child(
            div()
                .p_2()
                .child(selectable_styled_text(snapshot, block_index, context)),
        )
        .into_any_element()
}

fn render_special_block(
    label: &str,
    snapshot: InlineSnapshot,
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    let style = context.style;
    div()
        .w_full()
        .min_w_0()
        .my_1()
        .rounded(px(8.0))
        .border_1()
        .border_color(style.border_color)
        .bg(style.code_background.opacity(0.45))
        .overflow_x_scrollbar()
        .child(
            div()
                .px_2()
                .pt_1()
                .pb_0p5()
                .text_size(px(11.0))
                .line_height(px(14.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(style.muted_color)
                .child(label.to_string()),
        )
        .child(
            div()
                .p_2()
                .font_family("JetBrains Mono")
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .line_height(miaominal_settings::scaled_line_height(18.0))
                .text_color(style.text_color)
                .child(selectable_styled_text(snapshot, block_index, context)),
        )
        .into_any_element()
}

fn render_table(
    alignments: &[Alignment],
    rows: &[TableRow],
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    let style = context.style;
    let column_count = alignments
        .len()
        .max(rows.iter().map(|row| row.cells.len()).max().unwrap_or(0));
    if column_count == 0 {
        return div().into_any_element();
    }

    let mut grid = div()
        .w_full()
        .min_w_0()
        .my_1()
        .grid()
        .grid_cols(column_count.min(u16::MAX as usize) as u16)
        .rounded(px(8.0))
        .border_1()
        .border_color(style.border_color)
        .overflow_x_scrollbar()
        .text_size(miaominal_settings::FontSize::Body.scaled())
        .line_height(miaominal_settings::scaled_line_height(18.0));

    for (row_index, row) in rows.iter().enumerate() {
        for column in 0..column_count {
            let cell = row.cells.get(column).cloned().unwrap_or(InlineSnapshot {
                text: String::new(),
                highlights: Vec::new(),
            });
            grid = grid.child(render_table_cell(
                cell,
                alignments.get(column).copied().unwrap_or(Alignment::None),
                row.is_header,
                block_index
                    .saturating_mul(10_000)
                    .saturating_add(row_index.saturating_mul(100))
                    .saturating_add(column),
                context,
            ));
        }
    }

    grid.into_any_element()
}

fn render_table_cell(
    snapshot: InlineSnapshot,
    alignment: Alignment,
    is_header: bool,
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    let style = context.style;
    let mut cell = div()
        .min_w(px(90.0))
        .px_2()
        .py_1p5()
        .border_b_1()
        .border_r_1()
        .border_color(style.border_color)
        .text_color(if is_header {
            style.text_color
        } else {
            style.muted_color
        })
        .text_align(match alignment {
            Alignment::Left | Alignment::None => TextAlign::Left,
            Alignment::Center => TextAlign::Center,
            Alignment::Right => TextAlign::Right,
        });

    if is_header {
        cell = cell
            .bg(style.code_background.opacity(0.45))
            .font_weight(FontWeight::SEMIBOLD);
    }

    cell.child(selectable_styled_text(snapshot, block_index, context))
        .into_any_element()
}

fn cached_markdown_document(source: &str, style: MarkdownViewStyle) -> MarkdownDocument {
    if source.len() > MARKDOWN_CACHE_MAX_SOURCE_BYTES {
        return MarkdownRenderer::new(style).parse(source);
    }

    let key = MarkdownCacheKey {
        source_hash: hash_value(source),
        source_len: source.len(),
        style: MarkdownStyleKey::from(style),
    };
    let cache = markdown_document_cache();

    if let Some(document) = cache.lock().ok().and_then(|cache| cache.get(&key)) {
        return document;
    }

    let document = MarkdownRenderer::new(style).parse(source);
    if let Ok(mut cache) = cache.lock() {
        cache.insert(key, document.clone());
    }
    document
}

fn markdown_document_cache() -> &'static Mutex<MarkdownDocumentCache> {
    static CACHE: OnceLock<Mutex<MarkdownDocumentCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(MarkdownDocumentCache::default()))
}

fn cached_syntect_highlights(
    language: &str,
    code: &str,
) -> Option<Vec<(Range<usize>, HighlightStyle)>> {
    if code.len() > HIGHLIGHT_MAX_BYTES {
        return None;
    }

    let key = CodeHighlightCacheKey {
        language_hash: hash_value(language),
        language_len: language.len(),
        code_hash: hash_value(code),
        code_len: code.len(),
    };
    let cache = code_highlight_cache();

    if let Some(highlights) = cache.lock().ok().and_then(|cache| cache.get(&key)) {
        return Some(highlights);
    }

    let highlights = syntect_highlights_uncached(language, code).unwrap_or_default();
    if let Ok(mut cache) = cache.lock() {
        cache.insert(key, highlights.clone());
    }
    Some(highlights)
}

fn code_highlight_cache() -> &'static Mutex<CodeHighlightCache> {
    static CACHE: OnceLock<Mutex<CodeHighlightCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(CodeHighlightCache::default()))
}

impl From<MarkdownViewStyle> for MarkdownStyleKey {
    fn from(style: MarkdownViewStyle) -> Self {
        Self {
            text_color: style.text_color.into(),
            muted_color: style.muted_color.into(),
            link_color: style.link_color.into(),
            code_background: style.code_background.into(),
            border_color: style.border_color.into(),
        }
    }
}

impl From<Hsla> for ColorKey {
    fn from(color: Hsla) -> Self {
        Self {
            h: color.h.to_bits(),
            s: color.s.to_bits(),
            l: color.l.to_bits(),
            a: color.a.to_bits(),
        }
    }
}

fn hash_value<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn selectable_styled_text(
    snapshot: InlineSnapshot,
    block_index: usize,
    context: &MarkdownRenderContext<'_>,
) -> AnyElement {
    let selection_range = context.selection.and_then(|selection| {
        selection_range_for_block(selection, block_index, snapshot.text.len())
    });
    let selection_color = context.selection.map(|selection| selection.color);
    let styled = styled_text(snapshot.clone());

    if let Some(handlers) = context.selection_handlers {
        SelectableStyledText::new(
            block_index,
            snapshot.text,
            styled,
            selection_range,
            selection_color,
            handlers,
        )
        .into_any_element()
    } else {
        styled.into_any_element()
    }
}

struct SelectableStyledText {
    block_index: usize,
    text: String,
    styled: StyledText,
    selection_range: Option<Range<usize>>,
    selection_color: Option<Hsla>,
    on_start: Rc<dyn Fn(usize, usize, &mut Window, &mut App)>,
    on_update: Rc<dyn Fn(usize, usize, &mut Window, &mut App)>,
    on_finish: Rc<dyn Fn(usize, usize, &mut Window, &mut App)>,
}

impl SelectableStyledText {
    fn new(
        block_index: usize,
        text: String,
        styled: StyledText,
        selection_range: Option<Range<usize>>,
        selection_color: Option<Hsla>,
        handlers: &MarkdownTextSelectionHandlers,
    ) -> Self {
        Self {
            block_index,
            text,
            styled,
            selection_range,
            selection_color,
            on_start: handlers.on_start.clone(),
            on_update: handlers.on_update.clone(),
            on_finish: handlers.on_finish.clone(),
        }
    }
}

impl Element for SelectableStyledText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(SharedString::from(format!("markdown-selectable-text-{}", self.block_index)).into())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.styled.request_layout(None, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled
            .prepaint(None, inspector_id, bounds, state, window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        paint_text_selection(
            &self.styled,
            self.selection_range.clone(),
            self.selection_color,
            window,
        );

        self.styled
            .paint(None, inspector_id, _bounds, state, &mut (), window, cx);

        let layout = self.styled.layout().clone();
        let block_index = self.block_index;
        let text = self.text.clone();
        let on_start = self.on_start.clone();
        let on_update = self.on_update.clone();
        let on_finish = self.on_finish.clone();
        let hitbox_for_down = hitbox.clone();

        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox_for_down.is_hovered(window) {
                return;
            }
            let index = match layout.index_for_position(event.position) {
                Ok(index) | Err(index) => clamp_to_char_boundary(&text, index),
            };
            on_start(block_index, index, window, cx);
        });

        let layout = self.styled.layout().clone();
        let block_index = self.block_index;
        let text = self.text.clone();
        let on_update = on_update.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.pressed_button.is_none() {
                return;
            }
            let index = match layout.index_for_position(event.position) {
                Ok(index) | Err(index) => clamp_to_char_boundary(&text, index),
            };
            on_update(block_index, index, window, cx);
        });

        let layout = self.styled.layout().clone();
        let block_index = self.block_index;
        let text = self.text.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let index = match layout.index_for_position(event.position) {
                Ok(index) | Err(index) => clamp_to_char_boundary(&text, index),
            };
            on_finish(block_index, index, window, cx);
        });
    }
}

fn selection_range_for_block(
    selection: &MarkdownTextSelection,
    block_index: usize,
    block_len: usize,
) -> Option<Range<usize>> {
    if block_index < selection.start_block || block_index > selection.end_block {
        return None;
    }

    let start = if block_index == selection.start_block {
        selection.start_offset.min(block_len)
    } else {
        0
    };
    let end = if block_index == selection.end_block {
        selection.end_offset.min(block_len)
    } else {
        block_len
    };
    (start < end).then_some(start..end)
}

impl IntoElement for SelectableStyledText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

fn paint_text_selection(
    styled: &StyledText,
    selection_range: Option<Range<usize>>,
    selection_color: Option<Hsla>,
    window: &mut Window,
) {
    let Some(range) = selection_range else {
        return;
    };
    if range.is_empty() {
        return;
    }

    let color = selection_color.unwrap_or_else(|| gpui::hsla(0.58, 0.65, 0.58, 0.35));
    let layout = styled.layout();
    let line_height = layout.line_height();
    let mut cursor = range.start;

    while cursor < range.end {
        let Some(start) = layout.position_for_index(cursor) else {
            break;
        };
        let mut next = next_char_boundary(layout, cursor, range.end);
        while next < range.end {
            let Some(position) = layout.position_for_index(next) else {
                break;
            };
            if (f32::from(position.y) - f32::from(start.y)).abs() >= 0.5 {
                break;
            }
            next = next_char_boundary(layout, next, range.end);
        }

        let end = layout
            .position_for_index(next)
            .or_else(|| layout.position_for_index(range.end))
            .unwrap_or(start);
        let width = (f32::from(end.x) - f32::from(start.x)).abs().max(1.0);
        window.paint_quad(fill(
            Bounds::new(start, gpui::size(px(width), line_height)),
            Background::from(color),
        ));
        cursor = next;
    }
}

fn clamp_to_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn next_char_boundary(layout: &gpui::TextLayout, current: usize, end: usize) -> usize {
    let text = layout.text();
    let mut next = current.saturating_add(1).min(end);
    while next < end && !text.is_char_boundary(next) {
        next += 1;
    }
    next.min(end)
}

fn styled_text(snapshot: InlineSnapshot) -> StyledText {
    let text: SharedString = snapshot.text.clone().into();
    StyledText::new(text).with_highlights(valid_highlights(&snapshot.text, snapshot.highlights))
}

fn task_marker(checked: bool, style: MarkdownViewStyle) -> AnyElement {
    div()
        .w(px(22.0))
        .flex_none()
        .pt(px(3.0))
        .child(
            div()
                .size(px(13.0))
                .rounded(px(3.0))
                .border_1()
                .border_color(if checked {
                    style.link_color
                } else {
                    style.border_color
                })
                .bg(if checked {
                    style.link_color.opacity(0.18)
                } else {
                    style.code_background.opacity(0.3)
                })
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.0))
                .line_height(px(12.0))
                .text_color(style.link_color)
                .child(if checked { "x" } else { "" }),
        )
        .into_any_element()
}

fn blockquote_kind_label(kind: BlockQuoteKind) -> &'static str {
    match kind {
        BlockQuoteKind::Note => "NOTE",
        BlockQuoteKind::Tip => "TIP",
        BlockQuoteKind::Important => "IMPORTANT",
        BlockQuoteKind::Warning => "WARNING",
        BlockQuoteKind::Caution => "CAUTION",
    }
}

fn highlight_for_style(style: MarkdownViewStyle, kind: &InlineKind) -> HighlightStyle {
    match kind {
        InlineKind::Emphasis => HighlightStyle {
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        },
        InlineKind::Strong => HighlightStyle {
            color: Some(style.text_color),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        },
        InlineKind::Strikethrough => HighlightStyle {
            strikethrough: Some(Default::default()),
            ..Default::default()
        },
        InlineKind::Link(_) | InlineKind::Image(_) => HighlightStyle {
            color: Some(style.link_color),
            underline: Some(gpui::UnderlineStyle {
                color: Some(style.link_color.opacity(0.65)),
                thickness: px(1.0),
                ..Default::default()
            }),
            ..Default::default()
        },
        InlineKind::Superscript => HighlightStyle {
            color: Some(style.link_color),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        },
        InlineKind::Subscript => HighlightStyle {
            color: Some(style.muted_color),
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        },
    }
}

fn syntect_highlights_uncached(
    language: &str,
    code: &str,
) -> Option<Vec<(Range<usize>, HighlightStyle)>> {
    if code.len() > HIGHLIGHT_MAX_BYTES {
        return None;
    }

    let syntax_set = syntax_set();
    let syntax = syntax_for_language(syntax_set, language)?;
    let mut highlighter = HighlightLines::new(syntax, syntect_theme());
    let mut highlights = Vec::new();
    let mut offset = 0usize;

    for line in LinesWithEndings::from(code) {
        let ranges = highlighter.highlight_line(line, syntax_set).ok()?;
        for (style, text) in ranges {
            let len = text.len();
            let end = offset + len;
            if len > 0 && style.foreground.a > 0 {
                highlights.push((offset..end, syntect_style_to_gpui(style)));
            }
            offset = end;
        }
    }

    Some(highlights)
}

pub(crate) fn valid_highlights(
    text: &str,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    highlights
        .into_iter()
        .filter(|(range, _)| {
            range.start <= range.end
                && range.end <= text.len()
                && text.is_char_boundary(range.start)
                && text.is_char_boundary(range.end)
        })
        .collect()
}

fn syntax_for_language<'a>(
    syntax_set: &'a SyntaxSet,
    language: &str,
) -> Option<&'a SyntaxReference> {
    let normalized = language.trim().trim_start_matches('.').to_ascii_lowercase();
    let extension = match normalized.as_str() {
        "bash" | "shell" | "sh" | "zsh" => "sh",
        "javascript" | "js" | "jsx" => "js",
        "typescript" | "ts" | "tsx" => "ts",
        "python" | "py" => "py",
        "rust" | "rs" => "rs",
        "json" => "json",
        "css" => "css",
        "html" | "xml" => "html",
        "markdown" | "md" => "md",
        "mermaid" => "mermaid",
        _ => normalized.as_str(),
    };
    syntax_set
        .find_syntax_by_extension(extension)
        .or_else(|| syntax_set.find_syntax_by_token(&normalized))
}

fn syntect_style_to_gpui(style: SyntectStyle) -> HighlightStyle {
    HighlightStyle {
        color: Some(
            gpui::rgba(
                u32::from(style.foreground.r) << 24
                    | u32::from(style.foreground.g) << 16
                    | u32::from(style.foreground.b) << 8
                    | u32::from(style.foreground.a),
            )
            .into(),
        ),
        font_weight: style
            .font_style
            .contains(SyntectFontStyle::BOLD)
            .then_some(FontWeight::BOLD),
        font_style: style
            .font_style
            .contains(SyntectFontStyle::ITALIC)
            .then_some(FontStyle::Italic),
        underline: style
            .font_style
            .contains(SyntectFontStyle::UNDERLINE)
            .then_some(gpui::UnderlineStyle {
                color: None,
                thickness: px(1.0),
                ..Default::default()
            }),
        ..Default::default()
    }
}

fn syntax_set() -> &'static SyntaxSet {
    static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn syntect_theme() -> &'static Theme {
    static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();
    let themes = THEME_SET.get_or_init(ThemeSet::load_defaults);
    themes
        .themes
        .get("base16-ocean.dark")
        .or_else(|| themes.themes.values().next())
        .expect("syntect ships at least one default theme")
}
