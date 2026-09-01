use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Row, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::{
    Config, Term,
    cell::{Cell, Flags, Hyperlink},
    color::Colors,
};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb};
use gpui::{Font, FontFallbacks, Hsla, Rgba, font, rgb};
pub use miaominal_core::terminal::MIN_TERMINAL_COLUMNS;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

mod input;

pub use input::{
    SearchMatchKind, TerminalInputModes, TerminalKeyEvent, TerminalKeyPhase, TerminalNamedKey,
    encode_terminal_input, encode_terminal_named_key, sanitize_paste,
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
pub const SCROLLBACK_LINES: usize = 10_000;
const MIN_TERMINAL_TEXT_CONTRAST: f32 = 4.5;
const CONTRAST_MIX_STEPS: usize = 8;

pub fn terminal_font() -> Font {
    let mut f = font(settings::terminal_font_family());
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
    pub link: Option<Arc<str>>,
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
    pub cursor: TerminalCursorPosition,
    #[allow(dead_code)]
    pub default_fg: Hsla,
    pub default_bg: Hsla,
    pub focused_cursor: bool,
    #[allow(dead_code)]
    pub search_total: usize,
    #[allow(dead_code)]
    pub search_current: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalCursorPosition {
    /// Cursor line relative to the current viewport. This can be outside the
    /// visible range while the user is viewing scrollback.
    pub viewport_line: i32,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalLink {
    pub start_column: usize,
    pub end_column: usize,
    pub uri: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalSelectionKind {
    Simple,
    Block,
    Semantic,
    Lines,
}

impl TerminalSelectionKind {
    fn into_alacritty(self) -> SelectionType {
        match self {
            Self::Simple => SelectionType::Simple,
            Self::Block => SelectionType::Block,
            Self::Semantic => SelectionType::Semantic,
            Self::Lines => SelectionType::Lines,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalSelectionPurpose {
    Copy,
    FreeType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalFreeTypeTarget {
    pub line: i32,
    pub column: usize,
    pub side: Side,
}

impl TerminalFreeTypeTarget {
    pub const fn new(line: i32, column: usize, side: Side) -> Self {
        Self { line, column, side }
    }

    fn point(self) -> Point {
        Point::new(Line(self.line), Column(self.column))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalEditStep {
    Left(usize),
    Right(usize),
    Up(usize),
    Down(usize),
    Delete(usize),
}

impl TerminalEditStep {
    pub const fn count(self) -> usize {
        match self {
            Self::Left(count)
            | Self::Right(count)
            | Self::Up(count)
            | Self::Down(count)
            | Self::Delete(count) => count,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalFreeTypeDropPlan {
    pub steps: Vec<TerminalEditStep>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalFreeTypeDragPlan {
    pub delete_steps: Vec<TerminalEditStep>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSelectionDragPlan {
    pub delete_steps: Option<Vec<TerminalEditStep>>,
    pub text: String,
}

/// A plan tied to the terminal generation and input modes it was computed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalFreeTypePlan<T> {
    pub generation: u64,
    pub input_modes: TerminalInputModes,
    pub value: T,
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
    selection_purpose: Option<TerminalSelectionPurpose>,
    free_type_selection_bounds: Option<SelectionRange>,
}

struct ParserInput {
    bytes: Vec<u8>,
    completion: Option<mpsc::SyncSender<()>>,
}

struct TerminalShared {
    core: Mutex<TerminalCore>,
    input_sequence: Mutex<()>,
    dirty_generation: AtomicU64,
    pending_inputs: AtomicU64,
    snapshot_cache: Mutex<Option<CachedTerminalSnapshot>>,
}

struct CachedTerminalSnapshot {
    generation: u64,
    focused: bool,
    palette: TerminalPaletteKey,
    snapshot: Arc<TerminalSnapshot>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TerminalPaletteKey {
    default_fg: u32,
    default_bg: u32,
    cursor: u32,
    selection: u32,
    ansi: [u32; 16],
}

impl TerminalPaletteKey {
    fn current() -> Self {
        let palette = settings::current_theme().terminal;
        Self {
            default_fg: palette.default_fg,
            default_bg: palette.default_bg,
            cursor: palette.cursor,
            selection: palette.selection,
            ansi: palette.ansi,
        }
    }
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
            selection_purpose: None,
            free_type_selection_bounds: None,
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
        if self.term.selection.is_none() {
            self.selection_purpose = None;
            self.free_type_selection_bounds = None;
        }
    }

    pub fn resize(&mut self, columns: usize, screen_lines: usize) -> bool {
        let columns = columns.max(MIN_TERMINAL_COLUMNS);
        let screen_lines = screen_lines.max(1);
        if self.columns == columns && self.screen_lines == screen_lines {
            return false;
        }

        let old_columns = self.columns;
        let old_screen_lines = self.screen_lines;
        let was_scrolled = self.term.grid().display_offset() != 0;
        let is_alternate_screen = self.term.mode().contains(TermMode::ALT_SCREEN);

        if columns < old_columns {
            // Alacritty applies height changes before column reflow. Keep that order, but split
            // the operations so history created specifically by reflow can be identified.
            if screen_lines != old_screen_lines {
                self.term.resize(TerminalDimensions {
                    columns: old_columns,
                    screen_lines,
                });
            }

            let absorbable_lines = if was_scrolled || is_alternate_screen {
                0
            } else {
                self.trailing_clear_screen_lines()
            };

            // Give reflow enough temporary scrollback headroom to expose every row we could
            // absorb. Without this, a full history buffer evicts old rows while reflowing and the
            // net history size does not reveal how many new rows were created.
            if absorbable_lines != 0 {
                self.term
                    .grid_mut()
                    .update_history(SCROLLBACK_LINES.saturating_add(absorbable_lines));
            }
            let history_before_reflow = self.term.grid().history_size();

            self.term.resize(TerminalDimensions {
                columns,
                screen_lines,
            });

            let reflow_history_growth = self
                .term
                .grid()
                .history_size()
                .saturating_sub(history_before_reflow);
            let lines_to_absorb = reflow_history_growth.min(absorbable_lines);

            // Column reflow anchors the cursor in place and puts newly wrapped rows into
            // scrollback, even when the bottom of the viewport is empty. Temporarily growing the
            // viewport pulls only those rows back from history; restoring the requested height
            // then removes the same number of unused trailing rows.
            if lines_to_absorb != 0
                && let Some(expanded_lines) = screen_lines.checked_add(lines_to_absorb)
            {
                self.term.resize(TerminalDimensions {
                    columns,
                    screen_lines: expanded_lines,
                });
                self.term.resize(TerminalDimensions {
                    columns,
                    screen_lines,
                });
            }

            if absorbable_lines != 0 {
                self.term.grid_mut().update_history(SCROLLBACK_LINES);
            }
        } else {
            self.term.resize(TerminalDimensions {
                columns,
                screen_lines,
            });
        }

        self.columns = columns;
        self.screen_lines = screen_lines;

        true
    }

    fn trailing_clear_screen_lines(&self) -> usize {
        (0..self.term.screen_lines())
            .rev()
            .take_while(|line| self.term.grid()[Line(*line as i32)].is_clear())
            .count()
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
        if self.display_offset() != 0 {
            self.clear_free_type_selection();
        }
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
        if self.display_offset() != 0 {
            self.clear_free_type_selection();
        }
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
        let kind = if block {
            TerminalSelectionKind::Block
        } else {
            TerminalSelectionKind::Simple
        };
        self.start_selection_with_kind(line, column, side, kind);
    }

    pub fn start_selection_with_kind(
        &mut self,
        line: i32,
        column: usize,
        side: Side,
        kind: TerminalSelectionKind,
    ) {
        self.start_selection_with_purpose(
            TerminalFreeTypeTarget::new(line, column, side),
            kind,
            TerminalSelectionPurpose::Copy,
        );
    }

    fn start_selection_with_purpose(
        &mut self,
        target: TerminalFreeTypeTarget,
        kind: TerminalSelectionKind,
        purpose: TerminalSelectionPurpose,
    ) {
        self.term.selection = Some(Selection::new(
            kind.into_alacritty(),
            target.point(),
            target.side,
        ));
        self.selection_purpose = Some(purpose);
        self.free_type_selection_bounds = None;
    }

    pub fn start_free_type_selection(
        &mut self,
        line: i32,
        column: usize,
        side: Side,
        kind: TerminalSelectionKind,
    ) -> bool {
        let Some(target) = self.free_type_target(line, column, side) else {
            return false;
        };
        let Some(bounds) = self.free_type_bounds_for_target(target) else {
            return false;
        };

        self.start_selection_with_purpose(target, kind, TerminalSelectionPurpose::FreeType);
        self.free_type_selection_bounds = Some(bounds);
        true
    }

    pub fn update_selection(&mut self, line: i32, column: usize, side: Side) {
        let Some(selection) = self.term.selection.as_mut() else {
            return;
        };
        let mut point = Point::new(Line(line), Column(column));
        if self.selection_purpose == Some(TerminalSelectionPurpose::FreeType)
            && let Some(bounds) = self.free_type_selection_bounds
        {
            point = point.clamp(bounds.start, bounds.end);
        }
        selection.update(point, side);
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
        self.selection_purpose = None;
        self.free_type_selection_bounds = None;
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

    pub fn selection_purpose(&self) -> Option<TerminalSelectionPurpose> {
        self.term.selection.as_ref()?;
        self.selection_purpose
    }

    pub fn has_free_type_selection(&self) -> bool {
        self.selection_purpose() == Some(TerminalSelectionPurpose::FreeType) && self.has_selection()
    }

    pub fn free_type_selection_contains(&self, line: i32, column: usize) -> bool {
        let Some(range) = self.free_type_selection_range() else {
            return false;
        };
        range.contains(Point::new(Line(line), Column(column)))
    }

    pub fn selection_contains(&self, line: i32, column: usize) -> bool {
        let Some(range) = self
            .term
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(&self.term))
        else {
            return false;
        };
        range.contains(Point::new(Line(line), Column(column)))
    }

    pub fn clear_free_type_selection(&mut self) -> bool {
        if self.selection_purpose() != Some(TerminalSelectionPurpose::FreeType) {
            return false;
        }
        self.clear_selection();
        true
    }

    pub fn free_type_target(
        &self,
        line: i32,
        column: usize,
        side: Side,
    ) -> Option<TerminalFreeTypeTarget> {
        if !self.free_type_editing_available()
            || line < 0
            || line >= self.screen_lines as i32
            || column >= self.columns
        {
            return None;
        }

        let mut target = TerminalFreeTypeTarget::new(line, column, side);
        let flags = self.term.grid()[target.point()].flags;
        if flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
            if target.line + 1 >= self.screen_lines as i32 {
                return None;
            }
            target.line += 1;
            target.column = 0;
            target.side = Side::Left;
        } else if flags.contains(Flags::WIDE_CHAR_SPACER) && target.column > 0 {
            target.column -= 1;
            target.side = Side::Right;
        }

        let cursor = self.term.grid().cursor.point;
        if self.term.mode().contains(TermMode::ALT_SCREEN) {
            return Some(target);
        }
        let start = self.term.line_search_left(cursor);
        let end = self.term.line_search_right(cursor);
        (target.point() >= start && target.point() <= end).then_some(target)
    }

    fn free_type_editing_available(&self) -> bool {
        self.display_offset() == 0 && self.term.mode().contains(TermMode::SHOW_CURSOR)
    }

    pub fn free_type_cursor_plan(
        &self,
        line: i32,
        column: usize,
        side: Side,
    ) -> Option<Vec<TerminalEditStep>> {
        let target = self.free_type_target(line, column, side)?;
        self.navigation_steps_to(target)
    }

    pub fn free_type_delete_plan(&self) -> Option<Vec<TerminalEditStep>> {
        Some(self.free_type_drag_plan()?.delete_steps)
    }

    pub fn free_type_drag_plan(&self) -> Option<TerminalFreeTypeDragPlan> {
        if self.selection_purpose() != Some(TerminalSelectionPurpose::FreeType) {
            return None;
        }
        let (range, delete_count, text) = self.free_type_selection_details()?;
        let target =
            TerminalFreeTypeTarget::new(range.start.line.0, range.start.column.0, Side::Left);
        let mut delete_steps = self.navigation_steps_to(target)?;
        if delete_count != 0 {
            delete_steps.push(TerminalEditStep::Delete(delete_count));
        }
        Some(TerminalFreeTypeDragPlan { delete_steps, text })
    }

    pub fn selection_drag_plan(&self) -> Option<TerminalSelectionDragPlan> {
        match self.selection_purpose()? {
            TerminalSelectionPurpose::Copy => {
                let text = self
                    .selection_text()?
                    .trim_end_matches(['\r', '\n'])
                    .to_string();
                (!text.is_empty()).then_some(TerminalSelectionDragPlan {
                    delete_steps: None,
                    text,
                })
            }
            TerminalSelectionPurpose::FreeType => {
                let plan = self.free_type_drag_plan()?;
                Some(TerminalSelectionDragPlan {
                    delete_steps: Some(plan.delete_steps),
                    text: plan.text,
                })
            }
        }
    }

    pub fn free_type_collapse_plan(&self, collapse_to_end: bool) -> Option<Vec<TerminalEditStep>> {
        if self.selection_purpose() != Some(TerminalSelectionPurpose::FreeType) {
            return None;
        }
        let range = self.free_type_selection_range()?;
        let target = if collapse_to_end {
            TerminalFreeTypeTarget::new(range.end.line.0, range.end.column.0, Side::Right)
        } else {
            TerminalFreeTypeTarget::new(range.start.line.0, range.start.column.0, Side::Left)
        };
        self.navigation_steps_to(target)
    }

    pub fn free_type_drop_plan(
        &self,
        line: i32,
        column: usize,
        side: Side,
        duplicate: bool,
    ) -> Option<TerminalFreeTypeDropPlan> {
        if self.selection_purpose() != Some(TerminalSelectionPurpose::FreeType) {
            return None;
        }
        let target = self.free_type_target(line, column, side)?;
        let bounds = self.free_type_selection_bounds?;
        let target_in_selection_line =
            target.point() >= bounds.start && target.point() <= bounds.end;
        if !target_in_selection_line && !self.term.mode().contains(TermMode::ALT_SCREEN) {
            return None;
        }

        let (range, delete_count, text) = self.free_type_selection_details()?;
        if text.is_empty() {
            return None;
        }

        let line_start = bounds.start;
        let source_start = self.boundary_index(line_start, range.start, Side::Left);
        let source_end = self.boundary_index(line_start, range.end, Side::Right);
        let target_index = self.boundary_index(line_start, target.point(), target.side);
        if target_index > source_start && target_index < source_end {
            return None;
        }

        if duplicate {
            let steps = self.navigation_steps_to(target)?;
            return Some(TerminalFreeTypeDropPlan { steps, text });
        }

        if !target_in_selection_line {
            let source_target =
                TerminalFreeTypeTarget::new(range.start.line.0, range.start.column.0, Side::Left);
            let mut steps = self.navigation_steps_to(source_target)?;
            if delete_count != 0 {
                steps.push(TerminalEditStep::Delete(delete_count));
            }
            steps.extend(self.navigation_steps_from(source_target, target)?);
            return Some(TerminalFreeTypeDropPlan { steps, text });
        }

        if target_index == source_start || target_index == source_end {
            return None;
        }

        let source_target =
            TerminalFreeTypeTarget::new(range.start.line.0, range.start.column.0, Side::Left);
        let mut steps = self.navigation_steps_to(source_target)?;
        if delete_count != 0 {
            steps.push(TerminalEditStep::Delete(delete_count));
        }
        if target_index < source_start {
            steps.push(TerminalEditStep::Left(source_start - target_index));
        } else if target_index > source_end {
            steps.push(TerminalEditStep::Right(target_index - source_end));
        }
        Some(TerminalFreeTypeDropPlan { steps, text })
    }

    pub fn selection_drop_plan(
        &self,
        line: i32,
        column: usize,
        side: Side,
        duplicate: bool,
    ) -> Option<TerminalFreeTypeDropPlan> {
        match self.selection_purpose()? {
            TerminalSelectionPurpose::Copy => {
                let target = self.free_type_target(line, column, side)?;
                if self.selection_contains(target.line, target.column) {
                    return None;
                }
                let plan = self.selection_drag_plan()?;
                let steps = self.navigation_steps_to(target)?;
                Some(TerminalFreeTypeDropPlan {
                    steps,
                    text: plan.text,
                })
            }
            TerminalSelectionPurpose::FreeType => {
                self.free_type_drop_plan(line, column, side, duplicate)
            }
        }
    }

    fn free_type_bounds_for_target(
        &self,
        target: TerminalFreeTypeTarget,
    ) -> Option<SelectionRange> {
        let target = target.point();
        let origin = if self.term.mode().contains(TermMode::ALT_SCREEN) {
            target
        } else {
            self.term.grid().cursor.point
        };
        let start = self.term.line_search_left(origin);
        let end = self.term.line_search_right(origin);
        (target >= start && target <= end).then(|| SelectionRange::new(start, end, false))
    }

    fn free_type_selection_range(&self) -> Option<SelectionRange> {
        if self.selection_purpose() != Some(TerminalSelectionPurpose::FreeType)
            || !self.free_type_editing_available()
        {
            return None;
        }
        let range = self.term.selection.as_ref()?.to_range(&self.term)?;
        let bounds = self.free_type_selection_bounds?;
        if range.start < bounds.start || range.end > bounds.end {
            return None;
        }
        if self.term.mode().contains(TermMode::ALT_SCREEN) {
            return Some(range);
        }
        let cursor = self.term.grid().cursor.point;
        let start = self.term.line_search_left(cursor);
        let end = self.term.line_search_right(cursor);
        if range.start < start || range.end > end {
            return None;
        }
        Some(range)
    }

    fn free_type_selection_details(&self) -> Option<(SelectionRange, usize, String)> {
        let mut range = self.free_type_selection_range()?;
        let selection = self.term.selection.as_ref()?;
        if selection.ty == SelectionType::Lines {
            while range.end > range.start && self.cell_is_trimmable_blank(range.end) {
                range.end = self.previous_point(range.end)?;
            }
        }

        let delete_count = self.edit_units_inclusive(range.start, range.end);
        let mut text = if selection.ty == SelectionType::Lines {
            self.term.bounds_to_string(range.start, range.end)
        } else {
            self.term.selection_to_string()?
        };
        while text.ends_with(['\r', '\n']) {
            text.pop();
        }
        Some((range, delete_count, text))
    }

    fn navigation_steps_to(&self, target: TerminalFreeTypeTarget) -> Option<Vec<TerminalEditStep>> {
        let cursor = self.term.grid().cursor.point;
        self.navigation_steps_from(
            TerminalFreeTypeTarget::new(cursor.line.0, cursor.column.0, Side::Left),
            target,
        )
    }

    fn navigation_steps_from(
        &self,
        origin: TerminalFreeTypeTarget,
        target: TerminalFreeTypeTarget,
    ) -> Option<Vec<TerminalEditStep>> {
        let start = self.term.line_search_left(origin.point());
        let end = self.term.line_search_right(origin.point());
        if target.point() >= start
            && target.point() <= end
            && origin.point() >= start
            && origin.point() <= end
        {
            let cursor_index = self.boundary_index(start, origin.point(), origin.side);
            let target_index = self.boundary_index(start, target.point(), target.side);
            return Some(match target_index.cmp(&cursor_index) {
                std::cmp::Ordering::Less => {
                    vec![TerminalEditStep::Left(cursor_index - target_index)]
                }
                std::cmp::Ordering::Greater => {
                    vec![TerminalEditStep::Right(target_index - cursor_index)]
                }
                std::cmp::Ordering::Equal => Vec::new(),
            });
        }

        if self.term.mode().contains(TermMode::ALT_SCREEN) {
            let origin_start = Point::new(Line(origin.line), Column(0));
            let target_start = Point::new(Line(target.line), Column(0));
            let origin_index = self.boundary_index(origin_start, origin.point(), origin.side);
            let target_index = self.boundary_index(target_start, target.point(), target.side);
            let mut steps = Vec::with_capacity(3);
            if origin_index != 0 {
                steps.push(TerminalEditStep::Left(origin_index));
            }
            if target.line < origin.line {
                steps.push(TerminalEditStep::Up((origin.line - target.line) as usize));
            } else {
                steps.push(TerminalEditStep::Down((target.line - origin.line) as usize));
            }
            if target_index != 0 {
                steps.push(TerminalEditStep::Right(target_index));
            }
            return Some(steps);
        }

        None
    }

    fn boundary_index(&self, start: Point, point: Point, side: Side) -> usize {
        let mut current = start;
        let mut count = 0;
        while current < point {
            count += usize::from(self.cell_is_edit_unit(current));
            let Some(next) = self.next_point(current) else {
                return count;
            };
            current = next;
        }
        if current == point && side == Side::Right {
            count += usize::from(self.cell_is_edit_unit(current));
        }
        count
    }

    fn edit_units_inclusive(&self, start: Point, end: Point) -> usize {
        let mut current = start;
        let mut count = 0;
        loop {
            count += usize::from(self.cell_is_edit_unit(current));
            if current == end {
                break;
            }
            let Some(next) = self.next_point(current) else {
                break;
            };
            current = next;
        }
        count
    }

    fn cell_is_edit_unit(&self, point: Point) -> bool {
        !self.term.grid()[point]
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    }

    fn cell_is_trimmable_blank(&self, point: Point) -> bool {
        let cell = &self.term.grid()[point];
        cell.c == ' '
            && cell.zerowidth().is_none_or(|chars| chars.is_empty())
            && !cell.flags.contains(Flags::WRAPLINE)
    }

    fn next_point(&self, point: Point) -> Option<Point> {
        if point.column.0 + 1 < self.columns {
            Some(Point::new(point.line, point.column + 1))
        } else if point.line.0 + 1 < self.screen_lines as i32 {
            Some(Point::new(point.line + 1, Column(0)))
        } else {
            None
        }
    }

    fn previous_point(&self, point: Point) -> Option<Point> {
        if point.column.0 > 0 {
            Some(Point::new(point.line, point.column - 1))
        } else if point.line.0 > 0 {
            Some(Point::new(point.line - 1, Column(self.columns - 1)))
        } else {
            None
        }
    }

    /// Look up the hyperlink at a viewport-relative cell position directly in
    /// the grid, using OSC 8 metadata when present and falling back to visible
    /// URL detection on only the target row.
    pub fn link_at(&self, viewport_line: usize, column: usize) -> Option<TerminalLink> {
        if column >= self.columns || viewport_line >= self.screen_lines {
            return None;
        }

        let grid = self.term.grid();
        let grid_line = Line(viewport_line as i32 - grid.display_offset() as i32);
        let row = &grid[grid_line];

        if let Some(hyperlink) = row[Column(column)].hyperlink() {
            let uri = hyperlink.uri();
            let mut start_column = column;
            while start_column > 0
                && row[Column(start_column - 1)]
                    .hyperlink()
                    .is_some_and(|candidate| candidate.uri() == uri)
            {
                start_column -= 1;
            }

            let mut end_column = column + 1;
            while end_column < self.columns
                && row[Column(end_column)]
                    .hyperlink()
                    .is_some_and(|candidate| candidate.uri() == uri)
            {
                end_column += 1;
            }

            return Some(TerminalLink {
                start_column,
                end_column,
                uri: Arc::from(uri),
            });
        }

        let row_chars = visible_grid_row_chars(row, self.columns);
        detect_visible_urls(&row_chars)
            .into_iter()
            .find(|url| (url.start..url.end).contains(&column))
            .and_then(|url| {
                let overlaps_osc_8 =
                    (url.start..url.end).any(|column| row[Column(column)].hyperlink().is_some());
                if overlaps_osc_8 {
                    return None;
                }

                Some(TerminalLink {
                    start_column: url.start,
                    end_column: url.end,
                    uri: Arc::from(row_chars[url.start..url.end].iter().collect::<String>()),
                })
            })
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
        let target = search_match_display_offset(
            target_point.line.0,
            self.screen_lines,
            self.history_size(),
        );
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
        let cursor_position = TerminalCursorPosition {
            viewport_line: renderable.cursor.point.line.0 + display_offset,
            column: renderable.cursor.point.column.0,
        };
        let rendered_cursor = if focused
            && renderable.cursor.shape != CursorShape::Hidden
            && renderable.display_offset == 0
        {
            usize::try_from(cursor_position.viewport_line)
                .ok()
                .map(|line| (line, cursor_position.column))
        } else {
            None
        };

        let selection_range = renderable.selection;

        let mut hyperlinks: HashMap<Hyperlink, Arc<str>> = HashMap::new();
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

            let is_cursor = rendered_cursor == Some((line_index, column));
            let is_selected = selection_range
                .map(|range| range.contains(indexed.point))
                .unwrap_or(false);

            let link = indexed.cell.hyperlink().map(|hyperlink| {
                if let Some(uri) = hyperlinks.get(&hyperlink) {
                    uri.clone()
                } else {
                    let uri: Arc<str> = Arc::from(hyperlink.uri());
                    hyperlinks.insert(hyperlink, uri.clone());
                    uri
                }
            });
            let cell = build_cell(
                indexed.cell,
                renderable.colors,
                is_cursor,
                is_selected,
                default_fg,
                default_bg,
                link,
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
            cursor: cursor_position,
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

/// Convert an absolute terminal grid line into the display offset that places it at the
/// viewport center when scrollback bounds allow it.
fn search_match_display_offset(match_line: i32, screen_lines: usize, history_size: usize) -> usize {
    let viewport_center = screen_lines / 2;
    let desired_offset = if match_line < 0 {
        let history_distance = usize::try_from(match_line.unsigned_abs()).unwrap_or(usize::MAX);
        viewport_center.saturating_add(history_distance)
    } else {
        let visible_line = usize::try_from(match_line).unwrap_or(usize::MAX);
        viewport_center.saturating_sub(visible_line)
    };

    desired_offset.min(history_size)
}

impl TerminalState {
    pub fn new(columns: usize, screen_lines: usize) -> Self {
        let shared = Arc::new(TerminalShared {
            core: Mutex::new(TerminalCore::new(columns, screen_lines)),
            input_sequence: Mutex::new(()),
            dirty_generation: AtomicU64::new(0),
            pending_inputs: AtomicU64::new(0),
            snapshot_cache: Mutex::new(None),
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

        let _input_sequence = self
            .shared
            .input_sequence
            .lock()
            .unwrap_or_else(|error| error.into_inner());
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
        {
            let _input_sequence = self
                .shared
                .input_sequence
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.shared.pending_inputs.fetch_add(1, Ordering::AcqRel);
            let input = ParserInput {
                bytes: text.as_bytes().to_vec(),
                completion: Some(completion),
            };
            if let Err(error) = self.parser.sender.send(input) {
                parse_input(&self.shared, error.0);
            }
        }
        let _ = completed.recv();
    }

    pub fn resize(&mut self, columns: usize, screen_lines: usize) -> bool {
        let mut core = self
            .shared
            .core
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let changed = core.resize(columns, screen_lines);
        if changed {
            self.mark_dirty_locked();
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
        self.with_core_mut_and_mark(|core| core.scroll(scroll));
    }

    pub fn scroll_to_display_offset(&mut self, target_offset: usize) {
        self.with_core_mut_and_mark(|core| core.scroll_to_display_offset(target_offset));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.with_core_mut_and_mark(TerminalCore::scroll_to_bottom);
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
        self.with_core_mut_and_mark(|core| core.start_selection(line, column, side, block));
    }

    pub fn start_selection_with_kind(
        &mut self,
        line: i32,
        column: usize,
        side: Side,
        kind: TerminalSelectionKind,
    ) {
        self.with_core_mut_and_mark(|core| {
            core.start_selection_with_kind(line, column, side, kind)
        });
    }

    pub fn update_selection(&mut self, line: i32, column: usize, side: Side) {
        self.with_core_mut_and_mark(|core| core.update_selection(line, column, side));
    }

    pub fn start_free_type_selection(
        &mut self,
        line: i32,
        column: usize,
        side: Side,
        kind: TerminalSelectionKind,
    ) -> bool {
        self.with_core_mut_and_mark(|core| core.start_free_type_selection(line, column, side, kind))
    }

    pub fn clear_selection(&mut self) {
        self.with_core_mut_and_mark(TerminalCore::clear_selection);
    }

    pub fn has_selection(&self) -> bool {
        self.with_core(TerminalCore::has_selection)
    }

    pub fn selection_text(&self) -> Option<String> {
        self.with_core(TerminalCore::selection_text)
    }

    pub fn selection_purpose(&self) -> Option<TerminalSelectionPurpose> {
        self.with_core(TerminalCore::selection_purpose)
    }

    pub fn has_free_type_selection(&self) -> bool {
        self.with_core(TerminalCore::has_free_type_selection)
    }

    pub fn free_type_selection_contains(&self, line: i32, column: usize) -> bool {
        self.with_core(|core| core.free_type_selection_contains(line, column))
    }

    pub fn selection_contains(&self, line: i32, column: usize) -> bool {
        self.with_core(|core| core.selection_contains(line, column))
    }

    pub fn clear_free_type_selection(&mut self) -> bool {
        self.with_core_mut_and_mark(TerminalCore::clear_free_type_selection)
    }

    pub fn free_type_target(
        &self,
        line: i32,
        column: usize,
        side: Side,
    ) -> Option<TerminalFreeTypeTarget> {
        self.with_core(|core| core.free_type_target(line, column, side))
    }

    pub fn free_type_cursor_plan(
        &self,
        line: i32,
        column: usize,
        side: Side,
    ) -> Option<TerminalFreeTypePlan<Vec<TerminalEditStep>>> {
        self.stable_free_type_plan(|core| core.free_type_cursor_plan(line, column, side))
    }

    pub fn free_type_delete_plan(&self) -> Option<TerminalFreeTypePlan<Vec<TerminalEditStep>>> {
        self.stable_free_type_plan(TerminalCore::free_type_delete_plan)
    }

    pub fn free_type_drag_plan(&self) -> Option<TerminalFreeTypePlan<TerminalFreeTypeDragPlan>> {
        self.stable_free_type_plan(TerminalCore::free_type_drag_plan)
    }

    pub fn selection_drag_plan(&self) -> Option<TerminalFreeTypePlan<TerminalSelectionDragPlan>> {
        self.stable_free_type_plan(TerminalCore::selection_drag_plan)
    }

    pub fn free_type_collapse_plan(
        &self,
        collapse_to_end: bool,
    ) -> Option<TerminalFreeTypePlan<Vec<TerminalEditStep>>> {
        self.stable_free_type_plan(|core| core.free_type_collapse_plan(collapse_to_end))
    }

    pub fn free_type_drop_plan(
        &self,
        line: i32,
        column: usize,
        side: Side,
        duplicate: bool,
    ) -> Option<TerminalFreeTypePlan<TerminalFreeTypeDropPlan>> {
        self.stable_free_type_plan(|core| core.free_type_drop_plan(line, column, side, duplicate))
    }

    pub fn selection_drop_plan(
        &self,
        line: i32,
        column: usize,
        side: Side,
        duplicate: bool,
    ) -> Option<TerminalFreeTypePlan<TerminalFreeTypeDropPlan>> {
        self.stable_free_type_plan(|core| core.selection_drop_plan(line, column, side, duplicate))
    }

    /// Runs `commit` only while the planned generation is still current and no
    /// parser input can be inserted between validation and the outbound write.
    /// Returns `Ok(None)` when the caller should recompute the plan.
    pub fn commit_free_type_plan<R, E>(
        &mut self,
        expected_generation: u64,
        clear_selection: bool,
        commit: impl FnOnce() -> Result<R, E>,
    ) -> Result<Option<R>, E> {
        let _input_sequence = self
            .shared
            .input_sequence
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.has_pending_input() {
            return Ok(None);
        }

        let mut core = self
            .shared
            .core
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.has_pending_input() || self.generation() != expected_generation {
            return Ok(None);
        }

        let result = commit()?;
        if clear_selection && core.term.selection.is_some() {
            core.clear_selection();
            self.mark_dirty_locked();
        }
        Ok(Some(result))
    }

    pub fn link_at(&self, viewport_line: usize, column: usize) -> Option<TerminalLink> {
        self.with_core(|core| core.link_at(viewport_line, column))
    }

    pub fn set_search(&mut self, pattern: &str) -> Result<usize, String> {
        self.with_core_mut_and_mark(|core| core.set_search(pattern))
    }

    pub fn clear_search(&mut self) {
        self.with_core_mut_and_mark(TerminalCore::clear_search);
    }

    pub fn next_match(&mut self) {
        self.with_core_mut_and_mark(TerminalCore::next_match);
    }

    pub fn prev_match(&mut self) {
        self.with_core_mut_and_mark(TerminalCore::prev_match);
    }

    pub fn snapshot(&self, focused: bool) -> Arc<TerminalSnapshot> {
        let palette = TerminalPaletteKey::current();
        let core = self
            .shared
            .core
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let generation = self.shared.dirty_generation.load(Ordering::Acquire);

        let mut cache = self
            .shared
            .snapshot_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(cached) = cache.as_ref()
            && cached.generation == generation
            && cached.focused == focused
            && cached.palette == palette
        {
            return cached.snapshot.clone();
        }

        let snapshot = Arc::new(core.snapshot(focused));
        *cache = Some(CachedTerminalSnapshot {
            generation,
            focused,
            palette,
            snapshot: snapshot.clone(),
        });
        snapshot
    }

    fn with_core<R>(&self, f: impl FnOnce(&TerminalCore) -> R) -> R {
        let core = self
            .shared
            .core
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        f(&core)
    }

    fn stable_free_type_plan<R>(
        &self,
        plan: impl FnOnce(&TerminalCore) -> Option<R>,
    ) -> Option<TerminalFreeTypePlan<R>> {
        let _input_sequence = self
            .shared
            .input_sequence
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.has_pending_input() {
            return None;
        }

        let core = self
            .shared
            .core
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.has_pending_input() {
            return None;
        }

        let value = plan(&core)?;
        Some(TerminalFreeTypePlan {
            generation: self.generation(),
            input_modes: core.input_modes(),
            value,
        })
    }

    fn with_core_mut_and_mark<R>(&self, f: impl FnOnce(&mut TerminalCore) -> R) -> R {
        let mut core = self
            .shared
            .core
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let result = f(&mut core);
        self.mark_dirty_locked();
        result
    }

    fn mark_dirty_locked(&self) {
        self.shared.dirty_generation.fetch_add(1, Ordering::Release);
        *self
            .shared
            .snapshot_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
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
        shared.dirty_generation.fetch_add(1, Ordering::Release);
        *shared
            .snapshot_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        drop(core);
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
    link: Option<Arc<str>>,
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
        link,
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
fn visible_grid_row_chars(row: &Row<Cell>, columns: usize) -> Vec<char> {
    (0..columns)
        .map(|column| {
            let cell = &row[Column(column)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                || cell.flags.contains(Flags::HIDDEN)
                || cell.c == '\0'
            {
                ' '
            } else {
                cell.c
            }
        })
        .collect()
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

        for detected in detect_visible_urls(&row_chars) {
            let has_existing_link = row[detected.start..detected.end]
                .iter()
                .any(|cell| cell.link.is_some());
            if has_existing_link {
                continue;
            }

            let url: Arc<str> = Arc::from(
                row_chars[detected.start..detected.end]
                    .iter()
                    .collect::<String>(),
            );
            for cell in &mut row[detected.start..detected.end] {
                if !cell.spacer && cell.character != '\0' {
                    cell.link = Some(url.clone());
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DetectedUrl {
    start: usize,
    end: usize,
}

const HTTPS_SCHEME: &[char] = &['h', 't', 't', 'p', 's', ':', '/', '/'];
const HTTP_SCHEME: &[char] = &['h', 't', 't', 'p', ':', '/', '/'];

fn detect_visible_urls(chars: &[char]) -> Vec<DetectedUrl> {
    let mut urls = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        let scheme_len = if starts_with_chars(chars, index, HTTPS_SCHEME) {
            Some(HTTPS_SCHEME.len())
        } else if starts_with_chars(chars, index, HTTP_SCHEME) {
            Some(HTTP_SCHEME.len())
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
            urls.push(DetectedUrl { start: index, end });
            index = end;
        } else {
            index += 1;
        }
    }

    urls
}

fn starts_with_chars(chars: &[char], start: usize, prefix: &[char]) -> bool {
    chars.get(start..start.saturating_add(prefix.len())) == Some(prefix)
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
    use std::fmt::Write as _;

    const RESIZE_REFLOW_TEXT: &str = "Linux localhost 7.0.10-x64v3-xanmod1 #0~20260523.ga55d99c SMP PREEMPT_DYNAMIC Sat May 23 18:51:58 UTC x86_64";

    #[test]
    fn detect_visible_urls_stops_before_trailing_punctuation() {
        let chars: Vec<char> = "see https://example.com/pkg?x=1, thanks".chars().collect();
        let urls = detect_visible_urls(&chars);

        assert_eq!(urls.len(), 1);
        assert_eq!(
            chars[urls[0].start..urls[0].end].iter().collect::<String>(),
            "https://example.com/pkg?x=1"
        );
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
    fn search_match_display_offset_centers_and_clamps_grid_lines() {
        assert_eq!(search_match_display_offset(-8, 12, 40), 14);
        assert_eq!(search_match_display_offset(-40, 12, 40), 40);
        assert_eq!(search_match_display_offset(1, 12, 40), 5);
        assert_eq!(search_match_display_offset(9, 12, 40), 0);
        assert_eq!(search_match_display_offset(-2, 1, 40), 2);
        assert_eq!(search_match_display_offset(-8, 12, 0), 0);
    }

    #[test]
    fn search_navigation_keeps_the_current_match_visible_and_centers_when_possible() {
        let mut core = TerminalCore::new(20, 5);
        core.push_bytes(
            [
                "match-old",
                "filler-1",
                "filler-2",
                "match-middle",
                "filler-4",
                "filler-5",
                "match-late",
                "filler-7",
                "filler-8",
                "filler-9",
                "tail",
            ]
            .join("\r\n")
            .as_bytes(),
        );

        assert_eq!(core.set_search("match"), Ok(3));
        assert_eq!(assert_current_search_match_position(&mut core), 0);

        core.next_match();
        assert_eq!(
            assert_current_search_match_position(&mut core),
            core.screen_lines() as i32 / 2
        );

        core.next_match();
        assert_eq!(
            assert_current_search_match_position(&mut core),
            core.screen_lines() as i32 / 2
        );

        core.next_match();
        assert_eq!(assert_current_search_match_position(&mut core), 0);

        core.prev_match();
        assert_eq!(
            assert_current_search_match_position(&mut core),
            core.screen_lines() as i32 / 2
        );
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
    fn free_type_target_is_limited_to_cursor_wrapped_line_on_main_screen() {
        let width = MIN_TERMINAL_COLUMNS;
        let mut core = TerminalCore::new(width, 4);
        core.push_bytes("a".repeat(width + 1).as_bytes());

        assert!(core.free_type_target(0, 2, Side::Left).is_some());
        assert!(core.free_type_target(1, 0, Side::Left).is_some());
        assert_eq!(core.free_type_target(2, 0, Side::Left), None);
        assert_eq!(
            core.free_type_cursor_plan(0, 2, Side::Left),
            Some(vec![TerminalEditStep::Left(width - 1)])
        );
    }

    #[test]
    fn free_type_target_and_selection_support_other_rows_on_alternate_screen() {
        let mut core = TerminalCore::new(12, 4);
        core.push_bytes(b"\x1b[?1049h\x1b[1;1Halpha\x1b[3;4H");

        assert_eq!(
            core.free_type_target(0, 2, Side::Left),
            Some(TerminalFreeTypeTarget::new(0, 2, Side::Left))
        );
        assert_eq!(
            core.free_type_cursor_plan(0, 2, Side::Left),
            Some(vec![
                TerminalEditStep::Left(3),
                TerminalEditStep::Up(2),
                TerminalEditStep::Right(2),
            ])
        );

        assert!(core.start_free_type_selection(0, 1, Side::Left, TerminalSelectionKind::Simple,));
        core.update_selection(0, 3, Side::Right);
        assert_eq!(core.selection_text().as_deref(), Some("lph"));
        assert_eq!(
            core.free_type_delete_plan(),
            Some(vec![
                TerminalEditStep::Left(3),
                TerminalEditStep::Up(2),
                TerminalEditStep::Right(1),
                TerminalEditStep::Delete(3),
            ])
        );
    }

    #[test]
    fn free_type_is_unavailable_in_scrollback_but_available_with_mouse_reporting() {
        let mut core = TerminalCore::new(8, 3);
        core.push_bytes(b"one\r\ntwo\r\nthree\r\nfour");
        let cursor = core.term.grid().cursor.point;
        assert!(core.start_free_type_selection(
            cursor.line.0,
            cursor.column.0.saturating_sub(1),
            Side::Left,
            TerminalSelectionKind::Simple,
        ));
        core.update_selection(
            cursor.line.0,
            cursor.column.0.saturating_sub(1),
            Side::Right,
        );
        assert!(core.has_free_type_selection());
        core.scroll(TerminalScroll::Top);
        assert!(core.display_offset() > 0);
        assert!(!core.has_free_type_selection());
        assert_eq!(core.free_type_target(0, 0, Side::Left), None);

        core.scroll(TerminalScroll::Bottom);
        core.push_bytes(b"\x1b[?1000h");
        assert!(core.mouse_protocol().is_enabled());
        let cursor = core.term.grid().cursor.point;
        assert_eq!(
            core.free_type_target(cursor.line.0, cursor.column.0, Side::Left),
            Some(TerminalFreeTypeTarget::new(
                cursor.line.0,
                cursor.column.0,
                Side::Left,
            ))
        );

        core.push_bytes(b"\x1b[?25l");
        assert_eq!(
            core.free_type_target(cursor.line.0, cursor.column.0, Side::Left),
            None
        );
    }

    #[test]
    fn copy_selection_supports_words_and_lines_in_scrollback() {
        let mut core = TerminalCore::new(12, 3);
        core.push_bytes(b"alpha beta\r\nsecond line\r\nthird\r\nfourth");
        core.scroll(TerminalScroll::Top);

        let history_line = -(core.display_offset() as i32);
        assert!(history_line < 0);
        assert!(terminal_grid_row_text(&core, history_line).starts_with("alpha beta"));

        core.start_selection_with_kind(
            history_line,
            7,
            Side::Left,
            TerminalSelectionKind::Semantic,
        );
        assert_eq!(
            core.selection_purpose(),
            Some(TerminalSelectionPurpose::Copy)
        );
        assert_eq!(core.selection_text().as_deref(), Some("beta"));
        assert!(!core.has_free_type_selection());
        assert!(core.selection_contains(history_line, 7));
        assert_eq!(
            core.selection_drag_plan(),
            Some(TerminalSelectionDragPlan {
                delete_steps: None,
                text: "beta".to_string(),
            })
        );

        core.scroll(TerminalScroll::Bottom);
        let cursor = core.term.grid().cursor.point;
        let drop = core
            .selection_drop_plan(cursor.line.0, cursor.column.0, Side::Left, false)
            .expect("history copy selection should be insertable at the editable cursor line");
        assert_eq!(drop.text, "beta");
        assert_eq!(
            core.selection_purpose(),
            Some(TerminalSelectionPurpose::Copy)
        );

        core.start_selection_with_kind(history_line, 2, Side::Left, TerminalSelectionKind::Lines);
        assert_eq!(
            core.selection_text()
                .as_deref()
                .map(|text| text.trim_end_matches(['\r', '\n'])),
            Some("alpha beta")
        );
        assert_eq!(
            core.selection_purpose(),
            Some(TerminalSelectionPurpose::Copy)
        );
        assert_eq!(
            core.selection_drag_plan(),
            Some(TerminalSelectionDragPlan {
                delete_steps: None,
                text: "alpha beta".to_string(),
            })
        );
    }

    #[test]
    fn free_type_wide_and_combining_cells_count_as_single_edit_units() {
        let mut core = TerminalCore::new(12, 3);
        core.push_bytes("a界e\u{301}b".as_bytes());

        assert!(core.start_free_type_selection(0, 1, Side::Left, TerminalSelectionKind::Simple,));
        core.update_selection(0, 3, Side::Right);

        assert_eq!(core.selection_text().as_deref(), Some("界e\u{301}"));
        assert_eq!(
            core.free_type_delete_plan(),
            Some(vec![TerminalEditStep::Left(3), TerminalEditStep::Delete(2)])
        );
    }

    #[test]
    fn free_type_semantic_selection_selects_words_and_matching_brackets() {
        let mut core = TerminalCore::new(24, 3);
        core.push_bytes(b"echo (alpha) omega");

        assert!(core.start_free_type_selection(0, 7, Side::Left, TerminalSelectionKind::Semantic,));
        assert_eq!(core.selection_text().as_deref(), Some("alpha"));

        assert!(core.start_free_type_selection(0, 5, Side::Left, TerminalSelectionKind::Semantic,));
        assert_eq!(core.selection_text().as_deref(), Some("(alpha)"));
    }

    #[test]
    fn free_type_lines_selection_uses_wrapped_logical_line() {
        let width = MIN_TERMINAL_COLUMNS;
        let text = "a".repeat(width + 1);
        let mut core = TerminalCore::new(width, 4);
        core.push_bytes(text.as_bytes());

        assert!(core.start_free_type_selection(0, 2, Side::Left, TerminalSelectionKind::Lines,));
        let range = core
            .free_type_selection_range()
            .expect("lines selection should have a range");

        assert_eq!(range.start, Point::new(Line(0), Column(0)));
        assert_eq!(range.end.line, Line(1));
        assert_eq!(
            core.free_type_selection_details()
                .map(|(_, delete_count, text)| (delete_count, text)),
            Some((width + 1, text))
        );
    }

    #[test]
    fn free_type_lines_selection_trims_text_and_delete_range_together() {
        let mut core = TerminalCore::new(12, 3);
        core.push_bytes(b"abc   ");

        assert!(core.start_free_type_selection(0, 1, Side::Left, TerminalSelectionKind::Lines,));
        assert_eq!(
            core.free_type_selection_details()
                .map(|(_, delete_count, text)| (delete_count, text)),
            Some((3, "abc".to_string()))
        );
    }

    #[test]
    fn free_type_target_maps_leading_wide_spacer_to_next_row() {
        let width = MIN_TERMINAL_COLUMNS;
        let mut core = TerminalCore::new(width, 3);
        core.push_bytes(b"\x1b[?1049h\x1b[2;2H");
        core.term.grid_mut()[Line(0)][Column(width - 1)]
            .flags
            .insert(Flags::LEADING_WIDE_CHAR_SPACER | Flags::WRAPLINE);
        core.term.grid_mut()[Line(1)][Column(0)]
            .flags
            .insert(Flags::WIDE_CHAR);

        assert_eq!(
            core.free_type_target(0, width - 1, Side::Right),
            Some(TerminalFreeTypeTarget::new(1, 0, Side::Left))
        );
    }

    #[test]
    fn alternate_screen_free_type_supports_other_rows_and_preserves_wrapped_navigation() {
        let width = MIN_TERMINAL_COLUMNS;
        let mut core = TerminalCore::new(width, 5);
        core.push_bytes(b"\x1b[?1049h");
        core.push_bytes("a".repeat(width + 2).as_bytes());

        assert!(core.free_type_target(0, 1, Side::Left).is_some());
        assert!(core.free_type_target(1, 1, Side::Left).is_some());
        assert!(core.free_type_target(2, 1, Side::Left).is_some());
        assert_eq!(
            core.free_type_cursor_plan(0, 1, Side::Left),
            Some(vec![TerminalEditStep::Left(width + 1)])
        );
    }

    #[test]
    fn alternate_screen_drop_uses_wrapped_line_edit_unit_indexes() {
        let width = MIN_TERMINAL_COLUMNS;
        let mut core = TerminalCore::new(width, 4);
        core.push_bytes(b"\x1b[?1049h");
        core.push_bytes("a".repeat(width + 2).as_bytes());
        assert!(core.start_free_type_selection(0, 1, Side::Left, TerminalSelectionKind::Simple,));
        core.update_selection(0, 2, Side::Right);

        let moved = core
            .free_type_drop_plan(1, 1, Side::Left, false)
            .expect("wrapped-line move should be planned");
        assert_eq!(
            moved.steps,
            vec![
                TerminalEditStep::Left(width + 1),
                TerminalEditStep::Delete(2),
                TerminalEditStep::Right(width - 2),
            ]
        );
    }

    #[test]
    fn alternate_screen_free_type_drop_moves_and_copies_selection_to_another_row() {
        let mut core = TerminalCore::new(12, 4);
        core.push_bytes(b"\x1b[?1049h\x1b[1;1Halpha\x1b[3;4H");
        assert!(core.start_free_type_selection(0, 1, Side::Left, TerminalSelectionKind::Simple,));
        core.update_selection(0, 3, Side::Right);

        let duplicate = core
            .free_type_drop_plan(2, 1, Side::Left, true)
            .expect("cross-row copy should be planned on the alternate screen");
        assert_eq!(duplicate.text, "lph");
        assert_eq!(duplicate.steps, vec![TerminalEditStep::Left(2)]);

        let moved = core
            .free_type_drop_plan(2, 1, Side::Left, false)
            .expect("cross-row move should be planned on the alternate screen");
        assert_eq!(moved.text, "lph");
        assert_eq!(
            moved.steps,
            vec![
                TerminalEditStep::Left(3),
                TerminalEditStep::Up(2),
                TerminalEditStep::Right(1),
                TerminalEditStep::Delete(3),
                TerminalEditStep::Left(1),
                TerminalEditStep::Down(2),
                TerminalEditStep::Right(1),
            ]
        );
    }

    #[test]
    fn free_type_drop_plans_move_copy_and_reject_internal_targets() {
        let mut core = TerminalCore::new(12, 3);
        core.push_bytes(b"abcdef");
        assert!(core.start_free_type_selection(0, 1, Side::Left, TerminalSelectionKind::Simple,));
        core.update_selection(0, 2, Side::Right);
        assert_eq!(core.selection_text().as_deref(), Some("bc"));

        assert_eq!(core.free_type_drop_plan(0, 1, Side::Right, false), None);
        assert!(core.has_free_type_selection());

        let duplicate = core
            .free_type_drop_plan(0, 0, Side::Left, true)
            .expect("copy before selection should be allowed");
        assert_eq!(duplicate.steps, vec![TerminalEditStep::Left(6)]);
        assert_eq!(duplicate.text, "bc");
        assert!(core.has_free_type_selection());

        assert!(core.start_free_type_selection(0, 1, Side::Left, TerminalSelectionKind::Simple,));
        core.update_selection(0, 2, Side::Right);
        let duplicate_after = core
            .free_type_drop_plan(0, 2, Side::Right, true)
            .expect("copy at the trailing boundary should be allowed");
        assert_eq!(duplicate_after.steps, vec![TerminalEditStep::Left(3)]);
        assert_eq!(duplicate_after.text, "bc");

        assert!(core.start_free_type_selection(0, 1, Side::Left, TerminalSelectionKind::Simple,));
        core.update_selection(0, 2, Side::Right);
        let moved = core
            .free_type_drop_plan(0, 5, Side::Right, false)
            .expect("move after selection should be planned");
        assert_eq!(
            moved.steps,
            vec![
                TerminalEditStep::Left(5),
                TerminalEditStep::Delete(2),
                TerminalEditStep::Right(3),
            ]
        );
        assert_eq!(moved.text, "bc");

        assert!(core.start_free_type_selection(0, 2, Side::Left, TerminalSelectionKind::Simple,));
        core.update_selection(0, 3, Side::Right);
        let moved_before = core
            .free_type_drop_plan(0, 0, Side::Left, false)
            .expect("move before selection should be planned");
        assert_eq!(
            moved_before.steps,
            vec![
                TerminalEditStep::Left(4),
                TerminalEditStep::Delete(2),
                TerminalEditStep::Left(2),
            ]
        );
        assert_eq!(moved_before.text, "cd");
    }

    #[test]
    fn free_type_drag_plan_keeps_source_text_and_delete_steps_together() {
        let mut core = TerminalCore::new(12, 3);
        core.push_bytes(b"abcdef");
        assert!(core.start_free_type_selection(0, 1, Side::Left, TerminalSelectionKind::Simple,));
        core.update_selection(0, 2, Side::Right);

        assert_eq!(
            core.free_type_drag_plan(),
            Some(TerminalFreeTypeDragPlan {
                delete_steps: vec![TerminalEditStep::Left(5), TerminalEditStep::Delete(2)],
                text: "bc".to_string(),
            })
        );
        assert_eq!(
            core.selection_drag_plan(),
            Some(TerminalSelectionDragPlan {
                delete_steps: Some(vec![TerminalEditStep::Left(5), TerminalEditStep::Delete(2),]),
                text: "bc".to_string(),
            })
        );
        assert!(core.has_free_type_selection());
    }

    #[test]
    fn free_type_plan_commit_rejects_changed_or_pending_terminal_state() {
        let mut terminal = TerminalState::new(12, 3);
        terminal.push_text("abcdef");
        assert!(terminal.start_free_type_selection(
            0,
            1,
            Side::Left,
            TerminalSelectionKind::Simple,
        ));
        terminal.update_selection(0, 2, Side::Right);

        let plan = terminal
            .free_type_delete_plan()
            .expect("stable selection should produce a plan");
        terminal.push_text("g");
        let mut committed = false;
        let result: Result<Option<()>, ()> =
            terminal.commit_free_type_plan(plan.generation, true, || {
                committed = true;
                Ok(())
            });
        assert_eq!(result, Ok(None));
        assert!(!committed);

        terminal.shared.pending_inputs.store(1, Ordering::Release);
        assert!(terminal.free_type_delete_plan().is_none());
        terminal.shared.pending_inputs.store(0, Ordering::Release);
    }

    #[test]
    fn successful_free_type_plan_commit_clears_selection() {
        let mut terminal = TerminalState::new(12, 3);
        terminal.push_text("abcdef");
        assert!(terminal.start_free_type_selection(
            0,
            1,
            Side::Left,
            TerminalSelectionKind::Simple,
        ));
        terminal.update_selection(0, 2, Side::Right);
        let plan = terminal
            .free_type_delete_plan()
            .expect("stable selection should produce a plan");

        let result: Result<Option<()>, ()> =
            terminal.commit_free_type_plan(plan.generation, true, || Ok(()));
        assert_eq!(result, Ok(Some(())));
        assert!(!terminal.has_free_type_selection());
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

    #[test]
    fn snapshot_is_reused_until_terminal_generation_changes() {
        let terminal = TerminalState::default();
        let first = terminal.snapshot(false);
        let second = terminal.snapshot(false);

        assert!(Arc::ptr_eq(&first, &second));

        terminal.push_bytes(b"changed");
        wait_for_parser(&terminal);
        let changed = terminal.snapshot(false);

        assert!(!Arc::ptr_eq(&first, &changed));
    }

    #[test]
    fn snapshot_cursor_position_does_not_depend_on_focus() {
        let mut core = TerminalCore::new(12, 3);
        core.push_bytes(b"\x1b[2;4H");

        let focused = core.snapshot(true);
        let unfocused = core.snapshot(false);
        let expected = TerminalCursorPosition {
            viewport_line: 1,
            column: 3,
        };

        assert_eq!(focused.cursor, expected);
        assert_eq!(unfocused.cursor, expected);
        assert!(focused.cells[1][3].is_cursor);
        assert!(!unfocused.cells.iter().flatten().any(|cell| cell.is_cursor));
    }

    #[test]
    fn snapshot_cursor_position_can_be_below_a_scrolled_viewport() {
        let mut core = TerminalCore::new(12, 3);
        core.push_bytes(b"0\r\n1\r\n2\r\n3\r\n4\r\n5");
        core.scroll(TerminalScroll::Top);

        let snapshot = core.snapshot(true);

        assert!(snapshot.display_offset > 0);
        assert!(snapshot.cursor.viewport_line >= snapshot.screen_lines as i32);
        assert!(!snapshot.cells.iter().flatten().any(|cell| cell.is_cursor));
    }

    #[test]
    fn width_reflow_uses_trailing_blank_rows_before_scrollback() {
        let mut core = TerminalCore::new(120, 32);
        core.push_bytes(format!("{RESIZE_REFLOW_TEXT}\r\n$ ").as_bytes());

        assert_eq!(core.history_size(), 0);
        assert!(core.resize(60, 32));

        assert_eq!(core.history_size(), 0);
        assert_eq!(terminal_grid_row_text(&core, 0), text_chunk(0, 60));
        assert_eq!(terminal_grid_row_text(&core, 1), text_chunk(60, 60));
        assert_eq!(terminal_grid_row_text(&core, 2), "$");
        assert_eq!(
            core.term.grid().cursor.point,
            Point::new(Line(2), Column(2))
        );
    }

    #[test]
    fn width_reflow_keeps_an_unterminated_first_line_visible() {
        let mut core = TerminalCore::new(120, 32);
        core.push_bytes(RESIZE_REFLOW_TEXT.as_bytes());

        assert!(core.resize(60, 32));

        assert_eq!(core.history_size(), 0);
        assert_eq!(terminal_grid_row_text(&core, 0), text_chunk(0, 60));
        assert_eq!(terminal_grid_row_text(&core, 1), text_chunk(60, 60));
        assert_eq!(
            core.term.grid().cursor.point,
            Point::new(Line(1), Column(48))
        );
    }

    #[test]
    fn width_reflow_uses_blank_rows_after_height_shrink() {
        let mut core = TerminalCore::new(120, 8);
        core.push_bytes(format!("{RESIZE_REFLOW_TEXT}\r\n$ ").as_bytes());

        assert!(core.resize(60, 4));

        assert_eq!(core.history_size(), 0);
        assert_eq!(core.screen_lines(), 4);
        assert_eq!(terminal_grid_row_text(&core, 0), text_chunk(0, 60));
        assert_eq!(terminal_grid_row_text(&core, 1), text_chunk(60, 60));
        assert_eq!(terminal_grid_row_text(&core, 2), "$");
    }

    #[test]
    fn width_reflow_preserves_existing_history_when_blank_rows_are_available() {
        let mut core = TerminalCore::new(120, 6);
        core.push_bytes(
            b"old0\r\nold1\r\nold2\r\nold3\r\nold4\r\nold5\r\nold6\r\nold7\r\n\x1b[2J\x1b[H",
        );
        core.push_bytes(format!("{RESIZE_REFLOW_TEXT}\r\n$ ").as_bytes());
        let history_before_resize = core.history_size();

        assert!(history_before_resize > 0);
        assert!(core.resize(60, 6));

        assert_eq!(core.history_size(), history_before_resize);
        assert_eq!(terminal_grid_row_text(&core, 0), text_chunk(0, 60));
        assert_eq!(terminal_grid_row_text(&core, 1), text_chunk(60, 60));
        assert_eq!(terminal_grid_row_text(&core, 2), "$");
    }

    #[test]
    fn width_reflow_at_scrollback_limit_does_not_evict_old_history() {
        let mut core = TerminalCore::new(120, 6);
        fill_scrollback_to_limit(&mut core);
        core.push_bytes(b"\x1b[2J\x1b[H");
        core.push_bytes(format!("{RESIZE_REFLOW_TEXT}\r\n$ ").as_bytes());

        assert_eq!(core.history_size(), SCROLLBACK_LINES);
        let oldest_history_line = terminal_grid_row_text(&core, -(SCROLLBACK_LINES as i32));

        assert!(core.resize(60, 6));

        assert_eq!(core.history_size(), SCROLLBACK_LINES);
        assert_eq!(
            terminal_grid_row_text(&core, -(SCROLLBACK_LINES as i32)),
            oldest_history_line
        );
        assert_eq!(terminal_grid_row_text(&core, 0), text_chunk(0, 60));
        assert_eq!(terminal_grid_row_text(&core, 1), text_chunk(60, 60));
        assert_eq!(terminal_grid_row_text(&core, 2), "$");
    }

    #[test]
    fn width_reflow_near_scrollback_limit_observes_more_than_remaining_capacity() {
        let mut core = TerminalCore::new(120, 10);
        fill_scrollback_to_limit(&mut core);
        core.push_bytes(b"\x1b[2J\x1b[H");
        core.push_bytes(format!("{RESIZE_REFLOW_TEXT}\r\n$ ").as_bytes());
        core.term.grid_mut().update_history(SCROLLBACK_LINES - 1);
        core.term.grid_mut().update_history(SCROLLBACK_LINES);
        let oldest_history_line = terminal_grid_row_text(&core, -((SCROLLBACK_LINES - 1) as i32));

        assert_eq!(core.history_size(), SCROLLBACK_LINES - 1);
        assert!(core.resize(20, 10));

        assert_eq!(core.history_size(), SCROLLBACK_LINES - 1);
        assert_eq!(
            terminal_grid_row_text(&core, -((SCROLLBACK_LINES - 1) as i32)),
            oldest_history_line
        );
        assert_eq!(terminal_grid_row_text(&core, 6), "$");
    }

    #[test]
    fn width_reflow_keeps_real_overflow_in_history_when_screen_is_full() {
        let mut core = TerminalCore::new(60, 4);
        let logical_lines = [
            "1".repeat(40),
            "2".repeat(40),
            "3".repeat(40),
            "4".repeat(40),
        ];
        core.push_bytes(logical_lines.join("\r\n").as_bytes());

        assert_eq!(core.trailing_clear_screen_lines(), 0);
        assert!(core.resize(20, 4));

        assert!(core.history_size() > 0);
        assert_eq!(all_terminal_text(&core), logical_lines.concat());
    }

    #[test]
    fn width_reflow_can_absorb_multiple_wrapped_rows() {
        let mut core = TerminalCore::new(120, 10);
        core.push_bytes(format!("{RESIZE_REFLOW_TEXT}\r\n$ ").as_bytes());

        assert!(core.resize(20, 10));

        assert_eq!(core.history_size(), 0);
        assert_eq!(terminal_grid_row_text(&core, 6), "$");
        assert_eq!(
            core.term.grid().cursor.point,
            Point::new(Line(6), Column(2))
        );
    }

    #[test]
    fn width_reflow_does_not_reanchor_a_scrolled_viewport() {
        let mut core = TerminalCore::new(120, 6);
        core.push_bytes(
            b"old0\r\nold1\r\nold2\r\nold3\r\nold4\r\nold5\r\nold6\r\nold7\r\n\x1b[2J\x1b[H",
        );
        core.push_bytes(format!("{RESIZE_REFLOW_TEXT}\r\n$ ").as_bytes());
        core.scroll(TerminalScroll::Top);
        let history_before_resize = core.history_size();

        assert!(core.display_offset() > 0);
        assert!(core.resize(60, 6));

        assert!(core.display_offset() > 0);
        assert!(core.history_size() > history_before_resize);
    }

    #[test]
    fn link_at_detects_visible_url_without_snapshot() {
        let terminal = TerminalState::default();
        terminal.push_text("open https://example.test/path now");

        let link = terminal.link_at(0, 10).expect("expected visible URL");

        assert_eq!(link.start_column, 5);
        assert_eq!(link.end_column, 30);
        assert_eq!(link.uri.as_ref(), "https://example.test/path");
    }

    #[test]
    fn link_at_prefers_osc_8_hyperlink_span() {
        let terminal = TerminalState::default();
        terminal.push_text("\x1b]8;;https://example.test/target\x1b\\click\x1b]8;;\x1b\\");

        let link = terminal.link_at(0, 2).expect("expected OSC 8 link");

        assert_eq!(link.start_column, 0);
        assert_eq!(link.end_column, 5);
        assert_eq!(link.uri.as_ref(), "https://example.test/target");
    }

    #[test]
    fn link_at_suppresses_detected_url_when_osc_8_overlaps_its_span() {
        let terminal = TerminalState::default();
        terminal
            .push_text("https://\x1b]8;;https://osc.test/target\x1b\\example\x1b]8;;\x1b\\.test");

        assert_eq!(terminal.link_at(0, 1), None);

        let osc_link = terminal
            .link_at(0, 9)
            .expect("expected overlapping OSC 8 link");
        assert_eq!(osc_link.start_column, 8);
        assert_eq!(osc_link.end_column, 15);
        assert_eq!(osc_link.uri.as_ref(), "https://osc.test/target");
    }

    fn terminal_grid_row_text(core: &TerminalCore, line: i32) -> String {
        (0..core.term.columns())
            .map(|column| core.term.grid()[Line(line)][Column(column)].c)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn assert_current_search_match_position(core: &mut TerminalCore) -> i32 {
        let target = {
            let search = core
                .search
                .lock()
                .expect("search state should be available");
            let current = search.current.expect("search should have a current match");
            *search.matches[current].start()
        };
        let expected_offset =
            search_match_display_offset(target.line.0, core.screen_lines(), core.history_size());
        assert_eq!(core.display_offset(), expected_offset);

        let viewport_line = target.line.0 + core.display_offset() as i32;
        assert!((0..core.screen_lines() as i32).contains(&viewport_line));

        let snapshot = core.snapshot(false);
        assert!(
            snapshot
                .cells
                .iter()
                .flatten()
                .any(|cell| { cell.search_match == SearchMatchKind::Current })
        );
        viewport_line
    }

    fn text_chunk(start: usize, length: usize) -> String {
        RESIZE_REFLOW_TEXT
            .chars()
            .skip(start)
            .take(length)
            .collect()
    }

    fn all_terminal_text(core: &TerminalCore) -> String {
        let mut text = String::new();
        let topmost_line = -(core.history_size() as i32);
        for line in topmost_line..core.screen_lines() as i32 {
            text.push_str(&terminal_grid_row_text(core, line));
        }
        text
    }

    fn fill_scrollback_to_limit(core: &mut TerminalCore) {
        let input_lines = SCROLLBACK_LINES + core.screen_lines() - 1;
        let mut history_input = String::with_capacity(input_lines * 16);
        for index in 0..input_lines {
            write!(&mut history_input, "history-{index:05}\r\n").unwrap();
        }
        core.push_bytes(history_input.as_bytes());
        assert_eq!(core.history_size(), SCROLLBACK_LINES);
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
