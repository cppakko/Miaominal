use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::{Config, Term, cell::Cell, cell::Flags, color::Colors};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb};
use gpui::{Font, FontFallbacks, Hsla, Rgba, font, rgb};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

mod input;

pub use input::{
    SearchMatchKind, TerminalInputModes, TerminalKeyEvent, TerminalKeyPhase, encode_terminal_input,
    sanitize_paste,
};
use miaominal_settings as settings;

/// Maximum number of regex matches retained when running a scrollback search.
pub const MAX_SEARCH_MATCHES: usize = 1000;

/// Subset of alacritty events that the UI cares about. The PTY worker thread
/// produces these via [`MiaominalListener`]; the AppView drains them on the
/// foreground thread and reacts (OSC 52 clipboard writes, bell).
#[derive(Clone, Debug)]
pub enum TerminalEvent {
    ClipboardStore(String),
    Bell,
}

#[derive(Clone)]
pub struct MiaominalListener {
    sender: Sender<TerminalEvent>,
}

impl MiaominalListener {
    fn new(sender: Sender<TerminalEvent>) -> Self {
        Self { sender }
    }
}

impl EventListener for MiaominalListener {
    fn send_event(&self, event: Event) {
        let mapped = match event {
            Event::ClipboardStore(_, content) => Some(TerminalEvent::ClipboardStore(content)),
            Event::Bell => Some(TerminalEvent::Bell),
            _ => None,
        };
        if let Some(ev) = mapped {
            // Receiver disconnect just means the AppView is gone; drop silently.
            let _ = self.sender.send(ev);
        }
    }
}

pub const DEFAULT_TERMINAL_COLUMNS: usize = 120;
pub const DEFAULT_TERMINAL_LINES: usize = 32;
pub const MIN_TERMINAL_COLUMNS: usize = 20;
pub const SCROLLBACK_LINES: usize = 10_000;
const MIN_TERMINAL_TEXT_CONTRAST: f32 = 4.5;
const CONTRAST_MIX_STEPS: usize = 8;

pub fn terminal_font() -> Font {
    let mut f = font(settings::font_family());
    let fallbacks = settings::font_fallbacks();
    if !fallbacks.is_empty() {
        f.fallbacks = Some(FontFallbacks::from_fonts(fallbacks));
    }
    f
}

pub fn terminal_font_size() -> f32 {
    settings::font_size()
}

pub fn terminal_line_height_default() -> f32 {
    settings::line_height_default()
}

pub fn terminal_cell_width_default() -> f32 {
    settings::cell_width_default()
}

struct TerminalDimensions {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for TerminalDimensions {
    fn total_lines(&self) -> usize {
        self.screen_lines + SCROLLBACK_LINES
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

#[derive(Clone, Debug)]
pub struct TerminalCell {
    pub character: char,
    pub zero_width: Vec<char>,
    pub fg: Hsla,
    pub bg: Hsla,
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub wide: bool,
    pub spacer: bool,
    pub is_cursor: bool,
    pub link: Option<String>,
    pub search_match: SearchMatchKind,
}

impl TerminalCell {
    pub fn blank(fg: Hsla, bg: Hsla) -> Self {
        Self {
            character: ' ',
            zero_width: Vec::new(),
            fg,
            bg,
            bold: false,
            italic: false,
            dim: false,
            underline: false,
            strikethrough: false,
            wide: false,
            spacer: false,
            is_cursor: false,
            link: None,
            search_match: SearchMatchKind::None,
        }
    }
}

pub struct TerminalSnapshot {
    pub cells: Vec<Vec<TerminalCell>>,
    #[allow(dead_code)]
    pub columns: usize,
    pub screen_lines: usize,
    pub display_offset: usize,
    pub history_size: usize,
    #[allow(dead_code)]
    pub default_fg: Hsla,
    pub default_bg: Hsla,
    pub focused_cursor: bool,
    #[allow(dead_code)]
    pub search_total: usize,
    #[allow(dead_code)]
    pub search_current: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
pub enum TerminalScroll {
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

const PARSER_SLICE_BYTES: usize = 16 * 1024;

struct TerminalCore {
    term: Term<MiaominalListener>,
    parser: Processor,
    columns: usize,
    screen_lines: usize,
    events: Receiver<TerminalEvent>,
    search: Mutex<SearchState>,
}

struct ParserInput {
    bytes: Vec<u8>,
    completion: Option<mpsc::SyncSender<()>>,
}

struct TerminalShared {
    core: Mutex<TerminalCore>,
    dirty_generation: AtomicU64,
    pending_inputs: AtomicU64,
}

struct ParserHandle {
    sender: Sender<ParserInput>,
}

#[derive(Clone)]
pub struct TerminalState {
    shared: Arc<TerminalShared>,
    parser: Arc<ParserHandle>,
}

#[derive(Default)]
struct SearchState {
    pattern: Option<String>,
    matches: Vec<Match>,
    current: Option<usize>,
}

impl Default for TerminalState {
    fn default() -> Self {
        Self::new(DEFAULT_TERMINAL_COLUMNS, DEFAULT_TERMINAL_LINES)
    }
}

impl TerminalCore {
    fn new(columns: usize, screen_lines: usize) -> Self {
        let columns = columns.max(MIN_TERMINAL_COLUMNS);
        let screen_lines = screen_lines.max(1);
        let dimensions = TerminalDimensions {
            columns,
            screen_lines,
        };

        let config = Config {
            scrolling_history: SCROLLBACK_LINES,
            ..Default::default()
        };

        let (sender, receiver) = mpsc::channel();
        let listener = MiaominalListener::new(sender);

        Self {
            term: Term::new(config, &dimensions, listener),
            parser: Processor::new(),
            columns,
            screen_lines,
            events: receiver,
            search: Mutex::new(SearchState::default()),
        }
    }

    pub fn try_recv_event(&self) -> Option<TerminalEvent> {
        self.events.try_recv().ok()
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, columns: usize, screen_lines: usize) -> bool {
        let columns = columns.max(MIN_TERMINAL_COLUMNS);
        let screen_lines = screen_lines.max(1);
        if self.columns == columns && self.screen_lines == screen_lines {
            return false;
        }

        self.columns = columns;
        self.screen_lines = screen_lines;
        self.term.resize(TerminalDimensions {
            columns,
            screen_lines,
        });

        true
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    #[allow(dead_code)]
    pub fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    pub fn scroll(&mut self, scroll: TerminalScroll) {
        let scroll = match scroll {
            TerminalScroll::Lines(delta) => Scroll::Delta(delta),
            TerminalScroll::PageUp => Scroll::PageUp,
            TerminalScroll::PageDown => Scroll::PageDown,
            TerminalScroll::Top => Scroll::Top,
            TerminalScroll::Bottom => Scroll::Bottom,
        };
        self.term.scroll_display(scroll);
    }

    pub fn scroll_to_display_offset(&mut self, target_offset: usize) {
        let target_offset = target_offset.min(self.history_size());
        let current_offset = self.display_offset();
        if current_offset == target_offset {
            return;
        }

        let delta = target_offset as isize - current_offset as isize;
        let delta = match i32::try_from(delta) {
            Ok(delta) => delta,
            Err(_) if delta > 0 => i32::MAX,
            Err(_) => i32::MIN,
        };

        self.term.scroll_display(Scroll::Delta(delta));
    }

    pub fn scroll_to_bottom(&mut self) {
        if self.display_offset() != 0 {
            self.term.scroll_display(Scroll::Bottom);
        }
    }

    pub fn input_modes(&self) -> TerminalInputModes {
        let mode = self.term.mode();
        TerminalInputModes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            app_keypad: mode.contains(TermMode::APP_KEYPAD),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            focus_in_out: mode.contains(TermMode::FOCUS_IN_OUT),
            kitty_keyboard_protocol: mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL),
            kitty_disambiguate_escape_codes: mode.contains(TermMode::DISAMBIGUATE_ESC_CODES),
            kitty_report_event_types: mode.contains(TermMode::REPORT_EVENT_TYPES),
            kitty_report_alternate_keys: mode.contains(TermMode::REPORT_ALTERNATE_KEYS),
            kitty_report_all_keys_as_escape_codes: mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC),
            kitty_report_associated_text: mode.contains(TermMode::REPORT_ASSOCIATED_TEXT),
        }
    }

    pub fn bracketed_paste_enabled(&self) -> bool {
        self.input_modes().bracketed_paste
    }

    pub fn mouse_protocol(&self) -> MouseProtocol {
        let mode = self.term.mode();
        if mode.contains(TermMode::MOUSE_MOTION) {
            MouseProtocol::AnyEvent
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseProtocol::ButtonEvent
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseProtocol::Normal
        } else {
            MouseProtocol::Off
        }
    }

    pub fn mouse_encoding(&self) -> MouseEncoding {
        let mode = self.term.mode();
        if mode.contains(TermMode::SGR_MOUSE) {
            MouseEncoding::Sgr
        } else if mode.contains(TermMode::UTF8_MOUSE) {
            MouseEncoding::Utf8
        } else {
            MouseEncoding::Default
        }
    }

    pub fn alternate_scroll_active(&self) -> bool {
        self.term
            .mode()
            .contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
    }

    pub fn start_selection(&mut self, line: i32, column: usize, side: Side, block: bool) {
        let point = Point::new(Line(line), Column(column));
        let ty = if block {
            SelectionType::Block
        } else {
            SelectionType::Simple
        };
        self.term.selection = Some(Selection::new(ty, point, side));
    }

    pub fn update_selection(&mut self, line: i32, column: usize, side: Side) {
        let Some(selection) = self.term.selection.as_mut() else {
            return;
        };
        let point = Point::new(Line(line), Column(column));
        selection.update(point, side);
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn has_selection(&self) -> bool {
        self.term
            .selection
            .as_ref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    /// Look up the hyperlink URI at a viewport-relative cell position, using
    /// OSC 8 metadata when present and falling back to visible URL detection.
    pub fn link_at(&self, viewport_line: usize, column: usize) -> Option<String> {
        if column >= self.columns || viewport_line >= self.screen_lines {
            return None;
        }

        self.snapshot(false)
            .cells
            .get(viewport_line)
            .and_then(|row| row.get(column))
            .and_then(|cell| cell.link.clone())
    }

    /// Run a regex search across the entire scrollback grid. The pattern is
    /// pre-escaped by the caller when literal-mode searching is desired. Returns
    /// the number of matches found (capped at [`MAX_SEARCH_MATCHES`]). On
    /// success the first match is selected and the viewport scrolled to it.
    pub fn set_search(&mut self, pattern: &str) -> Result<usize, String> {
        if pattern.is_empty() {
            self.clear_search();
            return Ok(0);
        }
        let mut regex = RegexSearch::new(pattern).map_err(|err| err.to_string())?;
        let mut matches = Vec::new();
        let total_lines = self.term.grid().total_lines() as i32;
        let history = self.term.grid().history_size() as i32;
        let top_line = -history;
        let bottom_line = total_lines - history - 1;
        let mut start = Point::new(Line(top_line), Column(0));
        let bottom_right = Point::new(Line(bottom_line), Column(self.columns.saturating_sub(1)));
        while matches.len() < MAX_SEARCH_MATCHES {
            let Some(found) = self
                .term
                .regex_search_right(&mut regex, start, bottom_right)
            else {
                break;
            };
            let next_after = *found.end();
            matches.push(found);
            start = if next_after.column.0 + 1 < self.columns {
                Point::new(next_after.line, Column(next_after.column.0 + 1))
            } else if next_after.line < Line(bottom_line) {
                Point::new(next_after.line + 1, Column(0))
            } else {
                break;
            };
        }
        let total = matches.len();
        let current = if total == 0 { None } else { Some(0) };
        if let Ok(mut search) = self.search.lock() {
            search.pattern = Some(pattern.to_string());
            search.matches = matches;
            search.current = current;
        }
        if current.is_some() {
            self.scroll_to_current_match();
        }
        Ok(total)
    }

    pub fn clear_search(&mut self) {
        if let Ok(mut search) = self.search.lock() {
            search.pattern = None;
            search.matches.clear();
            search.current = None;
        }
    }

    pub fn next_match(&mut self) {
        {
            let Ok(mut search) = self.search.lock() else {
                return;
            };
            if search.matches.is_empty() {
                return;
            }
            let next = match search.current {
                Some(idx) => (idx + 1) % search.matches.len(),
                None => 0,
            };
            search.current = Some(next);
        }
        self.scroll_to_current_match();
    }

    pub fn prev_match(&mut self) {
        {
            let Ok(mut search) = self.search.lock() else {
                return;
            };
            if search.matches.is_empty() {
                return;
            }
            let prev = match search.current {
                Some(0) | None => search.matches.len() - 1,
                Some(idx) => idx - 1,
            };
            search.current = Some(prev);
        }
        self.scroll_to_current_match();
    }

    fn scroll_to_current_match(&mut self) {
        let target_point = {
            let Ok(search) = self.search.lock() else {
                return;
            };
            let Some(idx) = search.current else {
                return;
            };
            let Some(range) = search.matches.get(idx) else {
                return;
            };
            *range.start()
        };
        // Convert match line to a display offset that brings it on-screen.
        let line = target_point.line.0;
        if line >= 0 {
            self.scroll_to_display_offset(0);
            return;
        }
        let history = self.history_size() as i32;
        let desired_offset = (-line).min(history);
        // Position the match a couple rows from the top for context.
        let padding = (self.screen_lines as i32 / 4).max(1);
        let target = (desired_offset - padding).max(0).min(history) as usize;
        self.scroll_to_display_offset(target);
    }

    pub fn snapshot(&self, focused: bool) -> TerminalSnapshot {
        let columns = self.columns;
        let screen_lines = self.screen_lines;
        let default_fg = default_foreground();
        let default_bg = default_background();

        let mut cells = (0..screen_lines)
            .map(|_| {
                (0..columns)
                    .map(|_| TerminalCell::blank(default_fg, default_bg))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let renderable = self.term.renderable_content();
        let display_offset = renderable.display_offset as i32;
        let cursor = if focused
            && renderable.cursor.shape != CursorShape::Hidden
            && renderable.display_offset == 0
        {
            let viewport_line = renderable.cursor.point.line.0 + display_offset;
            usize::try_from(viewport_line)
                .ok()
                .map(|line| (line, renderable.cursor.point.column.0))
        } else {
            None
        };

        let selection_range = renderable.selection;

        for indexed in renderable.display_iter {
            let viewport_line = indexed.point.line.0 + display_offset;
            let Ok(line_index) = usize::try_from(viewport_line) else {
                continue;
            };

            if line_index >= screen_lines {
                continue;
            }

            let column = indexed.point.column.0;
            if column >= columns {
                continue;
            }

            let is_cursor = cursor == Some((line_index, column));
            let is_selected = selection_range
                .map(|range| range.contains(indexed.point))
                .unwrap_or(false);

            let cell = build_cell(
                indexed.cell,
                renderable.colors,
                is_cursor,
                is_selected,
                default_fg,
                default_bg,
            );
            cells[line_index][column] = cell;
        }

        apply_detected_links(&mut cells);

        let (search_total, search_current) =
            self.apply_search_highlights(&mut cells, display_offset);

        TerminalSnapshot {
            cells,
            columns,
            screen_lines,
            display_offset: renderable.display_offset,
            history_size: self.history_size(),
            default_fg,
            default_bg,
            focused_cursor: focused,
            search_total,
            search_current,
        }
    }

    fn apply_search_highlights(
        &self,
        cells: &mut [Vec<TerminalCell>],
        display_offset: i32,
    ) -> (usize, Option<usize>) {
        let Ok(search) = self.search.lock() else {
            return (0, None);
        };
        if search.matches.is_empty() {
            return (0, None);
        }
        let screen_lines = cells.len();
        let columns = cells.first().map(Vec::len).unwrap_or(0);
        for (idx, range) in search.matches.iter().enumerate() {
            let kind = if Some(idx) == search.current {
                SearchMatchKind::Current
            } else {
                SearchMatchKind::Match
            };
            let start = *range.start();
            let end = *range.end();
            let mut current = start;
            loop {
                let viewport_line = current.line.0 + display_offset;
                if let Ok(line_index) = usize::try_from(viewport_line)
                    && line_index < screen_lines
                    && current.column.0 < columns
                {
                    cells[line_index][current.column.0].search_match = kind;
                }
                if current == end {
                    break;
                }
                if current.column.0 + 1 < columns {
                    current.column.0 += 1;
                } else {
                    current.line.0 += 1;
                    current.column.0 = 0;
                }
                if current.line > end.line
                    || (current.line == end.line && current.column.0 > end.column.0)
                {
                    break;
                }
            }
        }
        (search.matches.len(), search.current)
    }
}

impl TerminalState {
    pub fn new(columns: usize, screen_lines: usize) -> Self {
        let shared = Arc::new(TerminalShared {
            core: Mutex::new(TerminalCore::new(columns, screen_lines)),
            dirty_generation: AtomicU64::new(0),
            pending_inputs: AtomicU64::new(0),
        });
        let (sender, receiver) = mpsc::channel::<ParserInput>();
        let parser = Arc::new(ParserHandle { sender });
        let worker_shared = shared.clone();

        thread::Builder::new()
            .name("miaominal-terminal-parser".to_string())
            .spawn(move || run_parser_loop(worker_shared, receiver))
            .expect("failed to spawn terminal parser thread");

        Self { shared, parser }
    }

    pub fn generation(&self) -> u64 {
        self.shared.dirty_generation.load(Ordering::Acquire)
    }

    pub fn has_pending_input(&self) -> bool {
        self.shared.pending_inputs.load(Ordering::Acquire) != 0
    }

    pub fn try_recv_event(&self) -> Option<TerminalEvent> {
        self.with_core(TerminalCore::try_recv_event)
    }

    pub fn push_bytes(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        self.shared.pending_inputs.fetch_add(1, Ordering::AcqRel);
        let input = ParserInput {
            bytes: bytes.to_vec(),
            completion: None,
        };
        if let Err(error) = self.parser.sender.send(input) {
            parse_input(&self.shared, error.0);
        }
    }

    pub fn push_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }

        let (completion, completed) = mpsc::sync_channel(1);
        self.shared.pending_inputs.fetch_add(1, Ordering::AcqRel);
        let input = ParserInput {
            bytes: text.as_bytes().to_vec(),
            completion: Some(completion),
        };
        if let Err(error) = self.parser.sender.send(input) {
            parse_input(&self.shared, error.0);
        }
        let _ = completed.recv();
    }

    pub fn resize(&mut self, columns: usize, screen_lines: usize) -> bool {
        let changed = self.with_core_mut(|core| core.resize(columns, screen_lines));
        if changed {
            self.mark_dirty();
        }
        changed
    }

    pub fn columns(&self) -> usize {
        self.with_core(TerminalCore::columns)
    }

    pub fn screen_lines(&self) -> usize {
        self.with_core(TerminalCore::screen_lines)
    }

    pub fn display_offset(&self) -> usize {
        self.with_core(TerminalCore::display_offset)
    }

    pub fn history_size(&self) -> usize {
        self.with_core(TerminalCore::history_size)
    }

    pub fn scroll(&mut self, scroll: TerminalScroll) {
        self.with_core_mut(|core| core.scroll(scroll));
        self.mark_dirty();
    }

    pub fn scroll_to_display_offset(&mut self, target_offset: usize) {
        self.with_core_mut(|core| core.scroll_to_display_offset(target_offset));
        self.mark_dirty();
    }

    pub fn scroll_to_bottom(&mut self) {
        self.with_core_mut(TerminalCore::scroll_to_bottom);
        self.mark_dirty();
    }

    pub fn input_modes(&self) -> TerminalInputModes {
        self.with_core(TerminalCore::input_modes)
    }

    pub fn bracketed_paste_enabled(&self) -> bool {
        self.with_core(TerminalCore::bracketed_paste_enabled)
    }

    pub fn mouse_protocol(&self) -> MouseProtocol {
        self.with_core(TerminalCore::mouse_protocol)
    }

    pub fn mouse_encoding(&self) -> MouseEncoding {
        self.with_core(TerminalCore::mouse_encoding)
    }

    pub fn alternate_scroll_active(&self) -> bool {
        self.with_core(TerminalCore::alternate_scroll_active)
    }

    pub fn start_selection(&mut self, line: i32, column: usize, side: Side, block: bool) {
        self.with_core_mut(|core| core.start_selection(line, column, side, block));
        self.mark_dirty();
    }

    pub fn update_selection(&mut self, line: i32, column: usize, side: Side) {
        self.with_core_mut(|core| core.update_selection(line, column, side));
        self.mark_dirty();
    }

    pub fn clear_selection(&mut self) {
        self.with_core_mut(TerminalCore::clear_selection);
        self.mark_dirty();
    }

    pub fn has_selection(&self) -> bool {
        self.with_core(TerminalCore::has_selection)
    }

    pub fn selection_text(&self) -> Option<String> {
        self.with_core(TerminalCore::selection_text)
    }

    pub fn link_at(&self, viewport_line: usize, column: usize) -> Option<String> {
        self.with_core(|core| core.link_at(viewport_line, column))
    }

    pub fn set_search(&mut self, pattern: &str) -> Result<usize, String> {
        let result = self.with_core_mut(|core| core.set_search(pattern));
        self.mark_dirty();
        result
    }

    pub fn clear_search(&mut self) {
        self.with_core_mut(TerminalCore::clear_search);
        self.mark_dirty();
    }

    pub fn next_match(&mut self) {
        self.with_core_mut(TerminalCore::next_match);
        self.mark_dirty();
    }

    pub fn prev_match(&mut self) {
        self.with_core_mut(TerminalCore::prev_match);
        self.mark_dirty();
    }

    pub fn snapshot(&self, focused: bool) -> TerminalSnapshot {
        self.with_core(|core| core.snapshot(focused))
    }

    fn with_core<R>(&self, f: impl FnOnce(&TerminalCore) -> R) -> R {
        let core = self
            .shared
            .core
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        f(&core)
    }

    fn with_core_mut<R>(&self, f: impl FnOnce(&mut TerminalCore) -> R) -> R {
        let mut core = self
            .shared
            .core
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        f(&mut core)
    }

    fn mark_dirty(&self) {
        self.shared.dirty_generation.fetch_add(1, Ordering::Release);
    }
}

fn run_parser_loop(shared: Arc<TerminalShared>, receiver: Receiver<ParserInput>) {
    while let Ok(input) = receiver.recv() {
        parse_input(&shared, input);
    }
}

fn parse_input(shared: &TerminalShared, input: ParserInput) {
    for bytes in input.bytes.chunks(PARSER_SLICE_BYTES) {
        let mut core = shared
            .core
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        core.push_bytes(bytes);
        drop(core);
        shared.dirty_generation.fetch_add(1, Ordering::Release);
    }
    shared.pending_inputs.fetch_sub(1, Ordering::AcqRel);
    if let Some(completion) = input.completion {
        let _ = completion.send(());
    }
}

fn build_cell(
    cell: &Cell,
    colors: &Colors,
    is_cursor: bool,
    is_selected: bool,
    default_fg: Hsla,
    default_bg: Hsla,
) -> TerminalCell {
    let mut fg = resolve_color(cell.fg, colors);
    let mut bg = resolve_color(cell.bg, colors);

    if cell.flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }

    if is_selected {
        bg = rgba_to_hsla(rgb(settings::current_theme().terminal.selection));
    }

    if is_cursor {
        let cursor_color = resolve_named_color(NamedColor::Cursor, colors);
        bg = cursor_color;
        fg = default_bg;
    }

    let character = if cell.flags.contains(Flags::HIDDEN) {
        ' '
    } else {
        cell.c
    };

    let zero_width = cell
        .zerowidth()
        .map(|chars| chars.to_vec())
        .unwrap_or_default();

    TerminalCell {
        character,
        zero_width,
        fg,
        bg,
        bold: cell.flags.contains(Flags::BOLD),
        italic: cell.flags.contains(Flags::ITALIC),
        dim: cell.flags.contains(Flags::DIM),
        underline: cell.flags.contains(Flags::UNDERLINE),
        strikethrough: cell.flags.contains(Flags::STRIKEOUT),
        wide: cell.flags.contains(Flags::WIDE_CHAR),
        spacer: cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
        is_cursor,
        link: cell
            .hyperlink()
            .map(|hyperlink| hyperlink.uri().to_string()),
        search_match: SearchMatchKind::None,
    }
    .with_default_fg(default_fg)
    .with_legible_foreground(default_fg, default_bg)
}

impl TerminalCell {
    fn with_default_fg(mut self, default_fg: Hsla) -> Self {
        if self.dim {
            self.fg = mix_with_default(self.fg, default_fg, 0.35);
        }
        self
    }

    fn with_legible_foreground(mut self, default_fg: Hsla, default_bg: Hsla) -> Self {
        self.fg = legible_foreground(self.fg, self.bg, default_fg, default_bg);
        self
    }
}
fn apply_detected_links(cells: &mut [Vec<TerminalCell>]) {
    for row in cells {
        let row_chars: Vec<char> = row
            .iter()
            .map(|cell| {
                if cell.spacer || cell.character == '\0' {
                    ' '
                } else {
                    cell.character
                }
            })
            .collect();

        for (start, end, url) in detect_visible_urls(&row_chars) {
            let has_existing_link = row[start..end].iter().any(|cell| cell.link.is_some());
            if has_existing_link {
                continue;
            }

            for cell in &mut row[start..end] {
                if !cell.spacer && cell.character != '\0' {
                    cell.link = Some(url.clone());
                }
            }
        }
    }
}

fn detect_visible_urls(chars: &[char]) -> Vec<(usize, usize, String)> {
    let mut urls = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        let scheme_len = if starts_with_chars(chars, index, "https://") {
            Some("https://".len())
        } else if starts_with_chars(chars, index, "http://") {
            Some("http://".len())
        } else {
            None
        };

        let Some(scheme_len) = scheme_len else {
            index += 1;
            continue;
        };

        let mut end = index + scheme_len;
        while end < chars.len() && is_url_char(chars[end]) {
            end += 1;
        }

        while end > index
            && matches!(
                chars[end - 1],
                '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}'
            )
        {
            end -= 1;
        }

        if end > index + scheme_len {
            let url = chars[index..end].iter().collect();
            urls.push((index, end, url));
            index = end;
        } else {
            index += 1;
        }
    }

    urls
}

fn starts_with_chars(chars: &[char], start: usize, prefix: &str) -> bool {
    let prefix_chars: Vec<char> = prefix.chars().collect();
    chars.get(start..start + prefix_chars.len()) == Some(prefix_chars.as_slice())
}

fn is_url_char(ch: char) -> bool {
    !ch.is_whitespace() && !matches!(ch, '"' | '<' | '>' | '`' | '{' | '}' | '|')
}

fn mix_with_default(color: Hsla, default: Hsla, amount: f32) -> Hsla {
    mix_colors(color, default, amount)
}

fn mix_colors(color: Hsla, target: Hsla, amount: f32) -> Hsla {
    let color: Rgba = color.into();
    let target: Rgba = target.into();
    let mix = Rgba {
        r: color.r + (target.r - color.r) * amount,
        g: color.g + (target.g - color.g) * amount,
        b: color.b + (target.b - color.b) * amount,
        a: color.a,
    };
    rgba_to_hsla(mix)
}

fn legible_foreground(fg: Hsla, bg: Hsla, default_fg: Hsla, default_bg: Hsla) -> Hsla {
    let current_ratio = contrast_ratio(fg, bg);
    if current_ratio >= MIN_TERMINAL_TEXT_CONTRAST {
        return fg;
    }

    let default_fg_ratio = contrast_ratio(default_fg, bg);
    let default_bg_ratio = contrast_ratio(default_bg, bg);
    let target = if default_fg_ratio >= default_bg_ratio {
        default_fg
    } else {
        default_bg
    };

    let mut best = fg;
    let mut best_ratio = current_ratio;
    for step in 1..=CONTRAST_MIX_STEPS {
        let amount = step as f32 / CONTRAST_MIX_STEPS as f32;
        let candidate = mix_colors(fg, target, amount);
        let ratio = contrast_ratio(candidate, bg);
        if ratio > best_ratio {
            best = candidate;
            best_ratio = ratio;
        }
        if ratio >= MIN_TERMINAL_TEXT_CONTRAST {
            return candidate;
        }
    }

    let black = rgba_to_hsla(Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    });
    let white = rgba_to_hsla(Rgba {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    });

    for candidate in [default_fg, default_bg, black, white] {
        let ratio = contrast_ratio(candidate, bg);
        if ratio > best_ratio {
            best = candidate;
            best_ratio = ratio;
        }
    }

    best
}

fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
    let a = relative_luminance(a);
    let b = relative_luminance(b);
    let lighter = a.max(b);
    let darker = a.min(b);
    (lighter + 0.05) / (darker + 0.05)
}

fn relative_luminance(color: Hsla) -> f32 {
    let color: Rgba = color.into();
    0.2126 * linear_channel(color.r)
        + 0.7152 * linear_channel(color.g)
        + 0.0722 * linear_channel(color.b)
}

fn linear_channel(value: f32) -> f32 {
    if value <= 0.03928 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn resolve_color(color: Color, colors: &Colors) -> Hsla {
    match color {
        Color::Named(named) => resolve_named_color(named, colors),
        Color::Spec(rgb_value) => rgba_to_hsla(rgb_to_rgba(rgb_value)),
        Color::Indexed(index) => {
            if let Some(rgb_value) = colors[index as usize] {
                rgba_to_hsla(rgb_to_rgba(rgb_value))
            } else {
                rgba_to_hsla(rgb_to_rgba(indexed_color(index)))
            }
        }
    }
}

fn resolve_named_color(named: NamedColor, colors: &Colors) -> Hsla {
    if let Some(rgb_value) = colors[named] {
        return rgba_to_hsla(rgb_to_rgba(rgb_value));
    }

    let palette = settings::current_theme().terminal;
    let ansi = palette.ansi;
    match named {
        NamedColor::Foreground | NamedColor::BrightForeground => default_foreground(),
        NamedColor::Background => default_background(),
        NamedColor::Cursor => rgba_to_hsla(rgb(palette.cursor)),
        NamedColor::Black => rgba_to_hsla(rgb(ansi[0])),
        NamedColor::Red => rgba_to_hsla(rgb(ansi[1])),
        NamedColor::Green => rgba_to_hsla(rgb(ansi[2])),
        NamedColor::Yellow => rgba_to_hsla(rgb(ansi[3])),
        NamedColor::Blue => rgba_to_hsla(rgb(ansi[4])),
        NamedColor::Magenta => rgba_to_hsla(rgb(ansi[5])),
        NamedColor::Cyan => rgba_to_hsla(rgb(ansi[6])),
        NamedColor::White => rgba_to_hsla(rgb(ansi[7])),
        NamedColor::BrightBlack => rgba_to_hsla(rgb(ansi[8])),
        NamedColor::BrightRed => rgba_to_hsla(rgb(ansi[9])),
        NamedColor::BrightGreen => rgba_to_hsla(rgb(ansi[10])),
        NamedColor::BrightYellow => rgba_to_hsla(rgb(ansi[11])),
        NamedColor::BrightBlue => rgba_to_hsla(rgb(ansi[12])),
        NamedColor::BrightMagenta => rgba_to_hsla(rgb(ansi[13])),
        NamedColor::BrightCyan => rgba_to_hsla(rgb(ansi[14])),
        NamedColor::BrightWhite => rgba_to_hsla(rgb(ansi[15])),
        NamedColor::DimBlack => dim_color(ansi[0]),
        NamedColor::DimRed => dim_color(ansi[1]),
        NamedColor::DimGreen => dim_color(ansi[2]),
        NamedColor::DimYellow => dim_color(ansi[3]),
        NamedColor::DimBlue => dim_color(ansi[4]),
        NamedColor::DimMagenta => dim_color(ansi[5]),
        NamedColor::DimCyan => dim_color(ansi[6]),
        NamedColor::DimWhite => dim_color(ansi[7]),
        NamedColor::DimForeground => dim_color(palette.default_fg),
    }
}

fn indexed_color(index: u8) -> Rgb {
    match index {
        0..=15 => {
            let palette = settings::current_theme().terminal;
            let [_, red, green, blue] = palette.ansi[index as usize].to_be_bytes();
            Rgb {
                r: red,
                g: green,
                b: blue,
            }
        }
        16..=231 => {
            let index = index - 16;
            let red = index / 36;
            let green = (index % 36) / 6;
            let blue = index % 6;

            Rgb {
                r: cube_value(red),
                g: cube_value(green),
                b: cube_value(blue),
            }
        }
        232..=255 => {
            let value = 8 + (index - 232) * 10;
            Rgb {
                r: value,
                g: value,
                b: value,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseProtocol {
    Off,
    Normal,
    ButtonEvent,
    AnyEvent,
}

impl MouseProtocol {
    pub fn is_enabled(self) -> bool {
        !matches!(self, MouseProtocol::Off)
    }

    pub fn reports_motion(self) -> bool {
        matches!(self, MouseProtocol::ButtonEvent | MouseProtocol::AnyEvent)
    }

    pub fn reports_motion_without_button(self) -> bool {
        matches!(self, MouseProtocol::AnyEvent)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEncoding {
    Default,
    Sgr,
    Utf8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseReportButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseReportKind {
    Press(MouseReportButton),
    Release(MouseReportButton),
    Motion(MouseReportButton),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MouseReportModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

pub fn encode_mouse_report(
    protocol: MouseProtocol,
    encoding: MouseEncoding,
    kind: MouseReportKind,
    column: usize,
    line: usize,
    modifiers: MouseReportModifiers,
) -> Option<Vec<u8>> {
    if !protocol.is_enabled() {
        return None;
    }

    let (button, is_motion, is_release) = match kind {
        MouseReportKind::Press(b) => (b, false, false),
        MouseReportKind::Release(b) => (b, false, true),
        MouseReportKind::Motion(b) => (b, true, false),
    };

    if is_motion && !protocol.reports_motion() {
        return None;
    }
    if is_motion
        && matches!(button, MouseReportButton::None)
        && !protocol.reports_motion_without_button()
    {
        return None;
    }
    if matches!(button, MouseReportButton::None) && !is_motion {
        return None;
    }

    let base = match button {
        MouseReportButton::Left => 0,
        MouseReportButton::Middle => 1,
        MouseReportButton::Right => 2,
        MouseReportButton::None => 3,
        MouseReportButton::WheelUp => 64,
        MouseReportButton::WheelDown => 65,
    };

    let mut cb = base;
    if modifiers.shift {
        cb |= 4;
    }
    if modifiers.alt {
        cb |= 8;
    }
    if modifiers.control {
        cb |= 16;
    }
    if is_motion {
        cb |= 32;
    }

    match encoding {
        MouseEncoding::Sgr => {
            let trailing = if is_release { b'm' } else { b'M' };
            let report = format!(
                "\x1b[<{};{};{}{}",
                cb,
                column + 1,
                line + 1,
                trailing as char
            );
            Some(report.into_bytes())
        }
        MouseEncoding::Default => {
            // For default encoding the released-button indicator is button code 3.
            let cb_default = if is_release && !is_motion {
                let mut released = 3u32;
                if modifiers.shift {
                    released |= 4;
                }
                if modifiers.alt {
                    released |= 8;
                }
                if modifiers.control {
                    released |= 16;
                }
                released
            } else {
                cb
            };
            let cb_byte = cb_default.checked_add(32)?;
            let cx_byte = (column as u32).checked_add(1)?.checked_add(32)?;
            let cy_byte = (line as u32).checked_add(1)?.checked_add(32)?;
            if cb_byte > 255 || cx_byte > 255 || cy_byte > 255 {
                return None;
            }
            Some(vec![
                0x1b,
                b'[',
                b'M',
                cb_byte as u8,
                cx_byte as u8,
                cy_byte as u8,
            ])
        }
        MouseEncoding::Utf8 => {
            let cb_default = if is_release && !is_motion {
                let mut released = 3u32;
                if modifiers.shift {
                    released |= 4;
                }
                if modifiers.alt {
                    released |= 8;
                }
                if modifiers.control {
                    released |= 16;
                }
                released
            } else {
                cb
            };
            let mut report = vec![0x1b, b'[', b'M'];
            push_utf8_mouse_byte(&mut report, cb_default + 32)?;
            push_utf8_mouse_byte(&mut report, (column as u32) + 1 + 32)?;
            push_utf8_mouse_byte(&mut report, (line as u32) + 1 + 32)?;
            Some(report)
        }
    }
}

fn push_utf8_mouse_byte(buffer: &mut Vec<u8>, value: u32) -> Option<()> {
    // UTF-8 mouse mode allows up to 2047 + 32.
    if value > 2047 {
        return None;
    }
    if value < 128 {
        buffer.push(value as u8);
    } else {
        let c = char::from_u32(value)?;
        let mut buf = [0u8; 4];
        let encoded = c.encode_utf8(&mut buf);
        buffer.extend_from_slice(encoded.as_bytes());
    }
    Some(())
}

fn cube_value(value: u8) -> u8 {
    if value == 0 { 0 } else { 55 + value * 40 }
}

fn dim_color(hex: u32) -> Hsla {
    let color = rgb(hex);
    rgba_to_hsla(Rgba {
        r: color.r * 0.7,
        g: color.g * 0.7,
        b: color.b * 0.7,
        a: color.a,
    })
}

pub fn default_foreground() -> Hsla {
    rgba_to_hsla(rgb(settings::current_theme().terminal.default_fg))
}

pub fn default_background() -> Hsla {
    rgba_to_hsla(rgb(settings::current_theme().terminal.default_bg))
}

fn rgb_to_rgba(rgb_value: Rgb) -> Rgba {
    Rgba {
        r: rgb_value.r as f32 / 255.0,
        g: rgb_value.g as f32 / 255.0,
        b: rgb_value.b as f32 / 255.0,
        a: 1.0,
    }
}

fn rgba_to_hsla(color: Rgba) -> Hsla {
    color.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_visible_urls_stops_before_trailing_punctuation() {
        let chars: Vec<char> = "see https://example.com/pkg?x=1, thanks".chars().collect();
        let urls = detect_visible_urls(&chars);

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].2, "https://example.com/pkg?x=1");
    }

    #[test]
    fn apply_detected_links_sets_link_on_visible_cells() {
        let fg = default_foreground();
        let bg = default_background();
        let mut row = "open http://example.test/path now"
            .chars()
            .map(|character| {
                let mut cell = TerminalCell::blank(fg, bg);
                cell.character = character;
                cell
            })
            .collect::<Vec<_>>();

        apply_detected_links(std::slice::from_mut(&mut row));

        let start = row
            .iter()
            .position(|cell| cell.character == 'h')
            .expect("expected visible URL start");
        let end = start + "http://example.test/path".chars().count();

        assert!(
            row[start..end]
                .iter()
                .all(|cell| cell.link.as_deref() == Some("http://example.test/path"))
        );
        assert_eq!(row[end].link, None);
    }

    #[test]
    fn legible_foreground_leaves_high_contrast_text_alone() {
        let fg = rgba_to_hsla(rgb(0xe8eaed));
        let bg = rgba_to_hsla(rgb(0x101418));
        let adjusted = legible_foreground(fg, bg, fg, bg);

        assert_rgba_close(adjusted, fg);
    }

    #[test]
    fn legible_foreground_repairs_low_contrast_ansi_blocks() {
        let fg = rgba_to_hsla(rgb(0xa5f29f));
        let bg = rgba_to_hsla(rgb(0x8ee58b));
        let default_fg = rgba_to_hsla(rgb(0xece6df));
        let default_bg = rgba_to_hsla(rgb(0x160f0b));

        let adjusted = legible_foreground(fg, bg, default_fg, default_bg);

        assert!(contrast_ratio(adjusted, bg) >= MIN_TERMINAL_TEXT_CONTRAST);
        assert!(contrast_ratio(adjusted, bg) > contrast_ratio(fg, bg));
    }

    #[test]
    fn alternate_scroll_requires_alternate_screen() {
        let terminal = TerminalState::default();

        assert!(!terminal.alternate_scroll_active());
    }

    #[test]
    fn alternate_scroll_is_active_in_alternate_screen() {
        let terminal = TerminalState::default();

        terminal.push_bytes(b"\x1b[?1049h");
        wait_for_parser(&terminal);

        assert!(terminal.alternate_scroll_active());
    }

    #[test]
    fn cloned_terminal_observes_background_parser_updates() {
        let terminal = TerminalState::default();
        let view = terminal.clone();
        let initial_generation = view.generation();

        terminal.push_bytes(b"background parser");
        wait_for_parser(&terminal);

        assert!(view.generation() > initial_generation);
        let first_line: String = view.snapshot(false).cells[0]
            .iter()
            .map(|cell| cell.character)
            .collect();
        assert!(first_line.starts_with("background parser"));
    }

    fn wait_for_parser(terminal: &TerminalState) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while terminal.has_pending_input() && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(!terminal.has_pending_input(), "terminal parser timed out");
    }

    fn assert_rgba_close(actual: Hsla, expected: Hsla) {
        let actual: Rgba = actual.into();
        let expected: Rgba = expected.into();
        let delta = (actual.r - expected.r).abs()
            + (actual.g - expected.g).abs()
            + (actual.b - expected.b).abs()
            + (actual.a - expected.a).abs();

        assert!(delta < 0.001, "color delta was {delta}");
    }
}
