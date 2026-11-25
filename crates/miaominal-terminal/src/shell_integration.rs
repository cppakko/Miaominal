use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};

const MAX_COMMAND_BLOCKS: usize = 500;
const MAX_OSC133_BODY_BYTES: usize = 512;
const MAX_CSI_PARAM_BYTES: usize = 32;

/// Execution status of an OSC 133 command block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandBlockStatus {
    Running,
    Finished(i32),
    FinishedUnknown,
}

/// Viewport-relative command marker emitted in [`super::TerminalSnapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCommandMarker {
    pub id: u64,
    /// Row in the current viewport (`0..screen_lines`) where the command prompt starts.
    pub viewport_line: usize,
    /// First row of this command block visible in the current viewport.
    pub visible_start_line: usize,
    /// Last row of this command block visible in the current viewport (inclusive).
    pub visible_end_line: usize,
    pub status: CommandBlockStatus,
    pub has_output: bool,
    pub command: String,
}

/// Extracted command text, output, and exit metadata for a command block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCommandBlockContent {
    pub id: u64,
    pub command: String,
    pub output: String,
    pub status: CommandBlockStatus,
    pub cwd: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Osc133Mark {
    PromptStart,
    CommandStart,
    CommandExecuted,
    CommandFinished { exit_code: Option<i32> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalControlEvent {
    Osc133(Osc133Mark),
    ClearBelow,
    ClearAbove,
    ClearAll,
    ClearSaved,
    Reset,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Osc133State {
    #[default]
    Ground,
    Escape,
    OscCode,
    OscCode1,
    OscCode13,
    OscCode133,
    Body,
    BodyEscape,
    CsiParams,
}

/// Incremental scanner for `OSC 133 ; <body> (BEL | ESC \)`, `CSI Ps J`, and `ESC c`.
///
/// Yields byte slices ending at each completed control boundary so the caller
/// advances the VTE parser up to the exact byte boundary before reacting.
#[derive(Default)]
pub(crate) struct Osc133Scanner {
    state: Osc133State,
    buffer: Vec<u8>,
}

fn emit_control_segment<'a>(
    bytes: &'a [u8],
    segment_start: &mut usize,
    esc_start: &mut Option<usize>,
    index: usize,
    event: TerminalControlEvent,
    on_segment: &mut impl FnMut(&'a [u8], Option<TerminalControlEvent>),
) {
    if let Some(start) = *esc_start
        && start > *segment_start
    {
        on_segment(&bytes[*segment_start..start], None);
        *segment_start = start;
    }
    on_segment(&bytes[*segment_start..=index], Some(event));
    *segment_start = index + 1;
    *esc_start = None;
}

impl Osc133Scanner {
    fn transition_after_escape(&mut self, byte: u8) -> Option<TerminalControlEvent> {
        match byte {
            b']' => {
                self.state = Osc133State::OscCode;
                None
            }
            b'[' => {
                self.buffer.clear();
                self.state = Osc133State::CsiParams;
                None
            }
            b'c' => {
                self.state = Osc133State::Ground;
                Some(TerminalControlEvent::Reset)
            }
            0x1b => {
                self.state = Osc133State::Escape;
                None
            }
            _ => {
                self.state = Osc133State::Ground;
                None
            }
        }
    }

    pub(crate) fn feed_segments<'a>(
        &mut self,
        bytes: &'a [u8],
        mut on_segment: impl FnMut(&'a [u8], Option<TerminalControlEvent>),
    ) {
        let mut segment_start = 0usize;
        let mut esc_start: Option<usize> = None;

        for (index, &byte) in bytes.iter().enumerate() {
            match self.state {
                Osc133State::Ground => {
                    if byte == 0x1b {
                        self.state = Osc133State::Escape;
                        esc_start = Some(index);
                    }
                }
                Osc133State::Escape => {
                    if byte == 0x1b {
                        esc_start = Some(index);
                    } else if let Some(event) = self.transition_after_escape(byte) {
                        emit_control_segment(
                            bytes,
                            &mut segment_start,
                            &mut esc_start,
                            index,
                            event,
                            &mut on_segment,
                        );
                    } else if matches!(self.state, Osc133State::Ground) {
                        esc_start = None;
                    }
                }
                Osc133State::OscCode => {
                    if byte == b'1' {
                        self.state = Osc133State::OscCode1;
                    } else if byte == 0x1b {
                        self.state = Osc133State::Escape;
                        esc_start = Some(index);
                    } else {
                        self.state = Osc133State::Ground;
                        esc_start = None;
                    }
                }
                Osc133State::OscCode1 => {
                    if byte == b'3' {
                        self.state = Osc133State::OscCode13;
                    } else if byte == 0x1b {
                        self.state = Osc133State::Escape;
                        esc_start = Some(index);
                    } else {
                        self.state = Osc133State::Ground;
                        esc_start = None;
                    }
                }
                Osc133State::OscCode13 => {
                    if byte == b'3' {
                        self.state = Osc133State::OscCode133;
                    } else if byte == 0x1b {
                        self.state = Osc133State::Escape;
                        esc_start = Some(index);
                    } else {
                        self.state = Osc133State::Ground;
                        esc_start = None;
                    }
                }
                Osc133State::OscCode133 => {
                    if byte == b';' {
                        self.buffer.clear();
                        self.state = Osc133State::Body;
                    } else if byte == 0x1b {
                        self.state = Osc133State::Escape;
                        esc_start = Some(index);
                    } else {
                        self.state = Osc133State::Ground;
                        esc_start = None;
                    }
                }
                Osc133State::Body => {
                    if byte == 0x07 {
                        let mark = parse_osc133_body(&self.buffer);
                        self.buffer.clear();
                        self.state = Osc133State::Ground;
                        esc_start = None;
                        if let Some(mark) = mark {
                            on_segment(
                                &bytes[segment_start..=index],
                                Some(TerminalControlEvent::Osc133(mark)),
                            );
                            segment_start = index + 1;
                        }
                    } else if byte == 0x1b {
                        self.state = Osc133State::BodyEscape;
                    } else if self.buffer.len() < MAX_OSC133_BODY_BYTES {
                        self.buffer.push(byte);
                    } else {
                        self.buffer.clear();
                        self.state = Osc133State::Ground;
                        esc_start = None;
                    }
                }
                Osc133State::BodyEscape => {
                    if byte == b'\\' {
                        let mark = parse_osc133_body(&self.buffer);
                        self.buffer.clear();
                        self.state = Osc133State::Ground;
                        esc_start = None;
                        if let Some(mark) = mark {
                            on_segment(
                                &bytes[segment_start..=index],
                                Some(TerminalControlEvent::Osc133(mark)),
                            );
                            segment_start = index + 1;
                        }
                    } else {
                        self.buffer.clear();
                        esc_start = if byte == 0x1b {
                            Some(index)
                        } else {
                            index.checked_sub(1)
                        };
                        if let Some(event) = self.transition_after_escape(byte) {
                            emit_control_segment(
                                bytes,
                                &mut segment_start,
                                &mut esc_start,
                                index,
                                event,
                                &mut on_segment,
                            );
                        } else if matches!(self.state, Osc133State::Ground) {
                            esc_start = None;
                        }
                    }
                }
                Osc133State::CsiParams => match byte {
                    b'0'..=b'9' | b';' | b':' => {
                        if self.buffer.len() < MAX_CSI_PARAM_BYTES {
                            self.buffer.push(byte);
                        } else {
                            self.buffer.clear();
                            self.state = Osc133State::Ground;
                            esc_start = None;
                        }
                    }
                    b'J' => {
                        let event = parse_csi_clear_event(&self.buffer);
                        self.buffer.clear();
                        self.state = Osc133State::Ground;
                        if let Some(event) = event {
                            emit_control_segment(
                                bytes,
                                &mut segment_start,
                                &mut esc_start,
                                index,
                                event,
                                &mut on_segment,
                            );
                        } else {
                            esc_start = None;
                        }
                    }
                    0x1b => {
                        self.buffer.clear();
                        self.state = Osc133State::Escape;
                        esc_start = Some(index);
                    }
                    _ => {
                        self.buffer.clear();
                        self.state = Osc133State::Ground;
                        esc_start = None;
                    }
                },
            }
        }

        if segment_start < bytes.len() {
            on_segment(&bytes[segment_start..], None);
        }
    }
}

fn parse_csi_clear_event(raw: &[u8]) -> Option<TerminalControlEvent> {
    let first_param = raw.split(|&b| b == b';' || b == b':').next().unwrap_or(raw);
    if first_param.is_empty() {
        return Some(TerminalControlEvent::ClearBelow);
    }
    let value = std::str::from_utf8(first_param).ok()?.parse::<u16>().ok()?;
    match value {
        0 => Some(TerminalControlEvent::ClearBelow),
        1 => Some(TerminalControlEvent::ClearAbove),
        2 => Some(TerminalControlEvent::ClearAll),
        3 => Some(TerminalControlEvent::ClearSaved),
        _ => None,
    }
}

fn parse_osc133_body(raw: &[u8]) -> Option<Osc133Mark> {
    let body = std::str::from_utf8(raw).ok()?.trim();
    if body.is_empty() {
        return None;
    }

    let mut parts = body.split(';');
    let code = parts.next()?.trim();
    match code {
        "A" => Some(Osc133Mark::PromptStart),
        "B" => Some(Osc133Mark::CommandStart),
        "C" => Some(Osc133Mark::CommandExecuted),
        "D" => {
            let mut exit_code = None;
            for part in parts {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                if let Ok(code) = part.parse::<i32>() {
                    exit_code = Some(code);
                    break;
                }
                if let Some(value) = part
                    .strip_prefix("err=")
                    .or_else(|| part.strip_prefix("exit="))
                    && let Ok(code) = value.trim().parse::<i32>()
                {
                    exit_code = Some(code);
                    break;
                }
            }
            Some(Osc133Mark::CommandFinished { exit_code })
        }
        _ => None,
    }
}

#[derive(Clone, Debug)]
struct CommandBlock {
    id: u64,
    prompt_line: i32,
    command_start: Option<(i32, usize)>,
    output_start_line: Option<i32>,
    end_line: i32,
    status: Option<CommandBlockStatus>,
    has_output: bool,
    command: String,
    cwd: Option<String>,
}

impl CommandBlock {
    fn new(id: u64, prompt_line: i32) -> Self {
        Self {
            id,
            prompt_line,
            command_start: None,
            output_start_line: None,
            end_line: prompt_line,
            status: None,
            has_output: false,
            command: String::new(),
            cwd: None,
        }
    }

    fn effective_end_line(&self, cursor_line: i32) -> i32 {
        match self.status {
            Some(CommandBlockStatus::Running) => cursor_line
                .max(self.output_start_line.unwrap_or(self.prompt_line))
                .max(self.prompt_line),
            _ => self.end_line.max(self.prompt_line),
        }
    }

    fn effective_has_output<T>(&self, term: &Term<T>) -> bool {
        let Some(output_start) = self.output_start_line else {
            return false;
        };
        let end_line = match self.status {
            Some(CommandBlockStatus::Running) => term.grid().cursor.point.line.0,
            _ => self.end_line,
        };
        if end_line < output_start {
            return false;
        }
        if matches!(self.status, Some(CommandBlockStatus::Running)) {
            !extract_lines(term, output_start, end_line).is_empty()
        } else {
            self.has_output
        }
    }
}

pub(crate) struct ShellIntegrationState {
    blocks: Vec<CommandBlock>,
    active_block_id: Option<u64>,
    next_id: u64,
    active_jump_block_id: Option<u64>,
    last_cwd: Option<String>,
}

impl Default for ShellIntegrationState {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            active_block_id: None,
            next_id: 1,
            active_jump_block_id: None,
            last_cwd: None,
        }
    }
}

impl ShellIntegrationState {
    pub(crate) fn set_cwd(&mut self, cwd: String) {
        self.last_cwd = Some(cwd);
    }

    pub(crate) fn clear_focused_command(&mut self) {
        self.active_jump_block_id = None;
    }

    pub(crate) fn focused_command_id(&self) -> Option<u64> {
        self.active_jump_block_id
    }

    pub(crate) fn has_command_blocks(&self) -> bool {
        self.blocks.iter().any(|block| block.status.is_some())
    }

    pub(crate) fn handle_event<T>(&mut self, term: &Term<T>, event: TerminalControlEvent) {
        match event {
            TerminalControlEvent::Osc133(mark) => self.handle_mark(term, mark),
            TerminalControlEvent::Reset => self.clear_all_blocks(),
            TerminalControlEvent::ClearAll => self.clear_viewport_blocks(term),
            TerminalControlEvent::ClearSaved => self.clear_scrollback_blocks(term),
            TerminalControlEvent::ClearBelow => self.clear_blocks_below_cursor(term),
            TerminalControlEvent::ClearAbove => self.clear_blocks_above_cursor(term),
        }
    }

    pub(crate) fn handle_mark<T>(&mut self, term: &Term<T>, mark: Osc133Mark) {
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        let cursor = term.grid().cursor.point;
        let cursor_line = cursor.line.0;
        let cursor_col = cursor.column.0;

        match mark {
            Osc133Mark::PromptStart => {
                self.close_or_discard_active_block(term, cursor_line);
                let id = self.allocate_id();
                self.blocks.push(CommandBlock::new(id, cursor_line));
                self.active_block_id = Some(id);
                self.enforce_block_cap();
            }
            Osc133Mark::CommandStart => {
                let active_needs_new = match self.active_block_mut() {
                    Some(block) if block.status.is_none() => {
                        block.command_start = Some((cursor_line, cursor_col));
                        false
                    }
                    Some(_) => true,
                    None => true,
                };
                if active_needs_new {
                    self.close_or_discard_active_block(term, cursor_line);
                    let id = self.allocate_id();
                    let mut block = CommandBlock::new(id, cursor_line);
                    block.command_start = Some((cursor_line, cursor_col));
                    self.blocks.push(block);
                    self.active_block_id = Some(id);
                    self.enforce_block_cap();
                }
            }
            Osc133Mark::CommandExecuted => {
                let needs_new = self
                    .active_block_mut()
                    .is_none_or(|block| block.status.is_some());
                if needs_new {
                    self.close_or_discard_active_block(term, cursor_line);
                    let id = self.allocate_id();
                    self.blocks.push(CommandBlock::new(id, cursor_line));
                    self.active_block_id = Some(id);
                    self.enforce_block_cap();
                }

                let cwd = self.last_cwd.clone();
                if let Some(block) = self.active_block_mut() {
                    let (cmd_end, output_start) = command_end_and_output_start(
                        block
                            .command_start
                            .map(|(line, _)| line)
                            .unwrap_or(block.prompt_line),
                        cursor_line,
                        cursor_col,
                        term.columns(),
                    );
                    if let Some((start_line, start_col)) = block.command_start {
                        block.command = extract_range(
                            term,
                            Point::new(Line(start_line), Column(start_col)),
                            cmd_end,
                        );
                    }
                    block.output_start_line = Some(output_start);
                    block.end_line = output_start.max(block.prompt_line);
                    block.status = Some(CommandBlockStatus::Running);
                    block.cwd = cwd;
                }
            }
            Osc133Mark::CommandFinished { exit_code } => {
                let Some(active_index) = self.active_block_index() else {
                    return;
                };
                let block = &mut self.blocks[active_index];
                let was_running = block.status.is_some();
                if !was_running {
                    // Some shells omit C on empty lines or simple commands.
                    if let Some((start_line, start_col)) = block.command_start {
                        let (cmd_end, _) = command_end_and_output_start(
                            start_line,
                            cursor_line,
                            cursor_col,
                            term.columns(),
                        );
                        block.command = extract_range(
                            term,
                            Point::new(Line(start_line), Column(start_col)),
                            cmd_end,
                        );
                    }
                    if block.command.trim().is_empty() {
                        // Discard empty prompt hits (e.g. pressing Enter on a blank prompt).
                        self.blocks.remove(active_index);
                        self.active_block_id = None;
                        return;
                    }
                    block.cwd = self.last_cwd.clone();
                }

                let status = exit_code.map_or(
                    CommandBlockStatus::FinishedUnknown,
                    CommandBlockStatus::Finished,
                );
                block.status = Some(status);
                if let Some(output_start) = block.output_start_line {
                    let end_line = if cursor_col == 0 {
                        cursor_line.saturating_sub(1)
                    } else {
                        cursor_line
                    };
                    block.end_line = end_line;
                    if end_line >= output_start {
                        block.has_output = !extract_lines(term, output_start, end_line).is_empty();
                    } else {
                        block.has_output = false;
                    }
                } else {
                    block.end_line = if cursor_col == 0 && cursor_line > block.prompt_line {
                        cursor_line - 1
                    } else {
                        cursor_line
                    };
                    block.has_output = false;
                }
                self.active_block_id = None;
            }
        }
    }

    pub(crate) fn shift_lines(&mut self, delta: i32, history_size: usize) {
        if delta != 0 {
            for block in &mut self.blocks {
                block.prompt_line -= delta;
                if let Some((line, _)) = &mut block.command_start {
                    *line -= delta;
                }
                if let Some(line) = &mut block.output_start_line {
                    *line -= delta;
                }
                block.end_line -= delta;
            }
        }
        self.prune_history(history_size);
    }

    pub(crate) fn prune_history(&mut self, history_size: usize) {
        let min_line = -(history_size as i32);
        let active_id = self.active_block_id;
        self.blocks.retain(|block| {
            block.end_line.max(block.prompt_line) >= min_line || Some(block.id) == active_id
        });
        if let Some(active_id) = active_id
            && let Some(block) = self.blocks.iter_mut().find(|b| b.id == active_id)
        {
            if block.prompt_line < min_line {
                block.prompt_line = min_line;
            }
            if let Some((line, _)) = &mut block.command_start
                && *line < min_line
            {
                *line = min_line;
            }
            if let Some(line) = &mut block.output_start_line
                && *line < min_line
            {
                *line = min_line;
            }
            if block.end_line < min_line {
                block.end_line = min_line;
            }
        }
        self.sync_referenced_ids();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub(crate) fn capture_reflow_anchors<T>(&self, term: &Term<T>) -> ReflowSnapshot {
        let cols = term.columns().max(1);
        let screen_lines = term.screen_lines();
        let topmost = -(term.grid().history_size() as i32);
        let bottom = screen_lines as i32 - 1;
        let spans = collect_logical_lines(term);

        let blocks = self
            .blocks
            .iter()
            .map(|block| BlockReflowAnchors {
                prompt: anchor_point(block.prompt_line, 0, &spans, cols, topmost, bottom),
                command_start: block
                    .command_start
                    .map(|(line, col)| anchor_point(line, col, &spans, cols, topmost, bottom)),
                output_start: block
                    .output_start_line
                    .map(|line| anchor_point(line, 0, &spans, cols, topmost, bottom)),
                end: anchor_end(block.end_line, &spans, cols, topmost, bottom),
            })
            .collect();

        ReflowSnapshot { blocks }
    }

    pub(crate) fn apply_reflow_anchors<T>(&mut self, term: &Term<T>, snapshot: ReflowSnapshot) {
        if snapshot.blocks.len() != self.blocks.len() {
            return;
        }
        let cols = term.columns().max(1);
        let screen_lines = term.screen_lines();
        let topmost = -(term.grid().history_size() as i32);
        let bottom = screen_lines as i32 - 1;
        let spans = collect_logical_lines(term);
        if spans.is_empty() {
            return;
        }

        for (block, anchors) in self.blocks.iter_mut().zip(snapshot.blocks) {
            let (prompt_line, _) = resolve_anchor(anchors.prompt, &spans, cols, topmost, bottom);
            block.prompt_line = prompt_line;
            block.command_start = anchors
                .command_start
                .map(|anchor| resolve_anchor(anchor, &spans, cols, topmost, bottom));
            block.output_start_line = anchors
                .output_start
                .map(|anchor| resolve_anchor(anchor, &spans, cols, topmost, bottom).0);
            let (end_line, _) = resolve_anchor(anchors.end, &spans, cols, topmost, bottom);
            block.end_line = end_line.max(block.prompt_line);
        }
    }

    pub(crate) fn markers_for_viewport<T>(
        &self,
        term: &Term<T>,
        screen_lines: usize,
        display_offset: usize,
    ) -> Vec<TerminalCommandMarker> {
        if screen_lines == 0 || term.mode().contains(TermMode::ALT_SCREEN) {
            return Vec::new();
        }
        let cursor_line = term.grid().cursor.point.line.0;
        let offset = display_offset as i32;
        let max_row = screen_lines as i32 - 1;

        self.blocks
            .iter()
            .filter_map(|block| {
                let status = block.status?;
                let viewport_prompt = block.prompt_line + offset;
                let viewport_line = usize::try_from(viewport_prompt).ok()?;
                if viewport_line >= screen_lines {
                    return None;
                }
                let end_line = block.effective_end_line(cursor_line);
                let viewport_end = (end_line + offset).clamp(viewport_prompt, max_row);
                let visible_end_line = usize::try_from(viewport_end).ok()?;

                Some(TerminalCommandMarker {
                    id: block.id,
                    viewport_line,
                    visible_start_line: viewport_line,
                    visible_end_line,
                    status,
                    has_output: block.effective_has_output(term),
                    command: block.command.clone(),
                })
            })
            .collect()
    }

    pub(crate) fn jump_previous(
        &mut self,
        screen_lines: usize,
        display_offset: usize,
        history_size: usize,
    ) -> Option<usize> {
        let executed_indices: Vec<usize> = self
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| block.status.map(|_| index))
            .collect();
        if executed_indices.is_empty() {
            return None;
        }

        let target_index = if let Some(current_id) = self.active_jump_block_id
            && let Some(pos) = executed_indices
                .iter()
                .position(|&index| self.blocks[index].id == current_id)
        {
            if pos == 0 {
                executed_indices[0]
            } else {
                executed_indices[pos - 1]
            }
        } else if display_offset == 0 {
            *executed_indices.last()?
        } else {
            let viewport_top = -(display_offset as i32);
            let viewport_bottom = viewport_top + screen_lines as i32 - 1;
            executed_indices
                .iter()
                .copied()
                .rfind(|&index| self.blocks[index].prompt_line < viewport_top)
                .or_else(|| {
                    executed_indices
                        .iter()
                        .copied()
                        .rfind(|&index| self.blocks[index].prompt_line <= viewport_bottom)
                })
                .unwrap_or(executed_indices[0])
        };

        let block = &self.blocks[target_index];
        self.active_jump_block_id = Some(block.id);
        Some(display_offset_for_prompt_line(
            block.prompt_line,
            screen_lines,
            display_offset,
            history_size,
        ))
    }

    pub(crate) fn jump_next(
        &mut self,
        screen_lines: usize,
        display_offset: usize,
        history_size: usize,
    ) -> Option<usize> {
        let executed_indices: Vec<usize> = self
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| block.status.map(|_| index))
            .collect();
        if executed_indices.is_empty() {
            return None;
        }

        if let Some(current_id) = self.active_jump_block_id
            && let Some(pos) = executed_indices
                .iter()
                .position(|&index| self.blocks[index].id == current_id)
        {
            if pos + 1 < executed_indices.len() {
                let block = &self.blocks[executed_indices[pos + 1]];
                self.active_jump_block_id = Some(block.id);
                return Some(display_offset_for_prompt_line(
                    block.prompt_line,
                    screen_lines,
                    display_offset,
                    history_size,
                ));
            }
            self.active_jump_block_id = None;
            return Some(0);
        }

        if display_offset == 0 {
            return None;
        }

        let viewport_top = -(display_offset as i32);
        if let Some(&target_index) = executed_indices
            .iter()
            .find(|&&index| self.blocks[index].prompt_line > viewport_top)
        {
            let block = &self.blocks[target_index];
            self.active_jump_block_id = Some(block.id);
            Some(display_offset_for_prompt_line(
                block.prompt_line,
                screen_lines,
                display_offset,
                history_size,
            ))
        } else {
            self.active_jump_block_id = None;
            Some(0)
        }
    }

    pub(crate) fn extract_block_by_id<T>(
        &self,
        term: &Term<T>,
        block_id: u64,
    ) -> Option<TerminalCommandBlockContent> {
        let block = self
            .blocks
            .iter()
            .find(|block| block.id == block_id && block.status.is_some())?;
        Some(self.build_block_content(term, block))
    }

    pub(crate) fn extract_block_at_grid_line<T>(
        &self,
        term: &Term<T>,
        grid_line: i32,
    ) -> Option<TerminalCommandBlockContent> {
        let cursor_line = term.grid().cursor.point.line.0;
        let block = self.blocks.iter().rfind(|block| {
            block.status.is_some()
                && grid_line >= block.prompt_line
                && grid_line <= block.effective_end_line(cursor_line)
        })?;
        Some(self.build_block_content(term, block))
    }

    pub(crate) fn extract_latest_block<T>(
        &self,
        term: &Term<T>,
    ) -> Option<TerminalCommandBlockContent> {
        let block = self.blocks.iter().rfind(|block| block.status.is_some())?;
        Some(self.build_block_content(term, block))
    }

    fn build_block_content<T>(
        &self,
        term: &Term<T>,
        block: &CommandBlock,
    ) -> TerminalCommandBlockContent {
        let cursor_line = term.grid().cursor.point.line.0;
        let output = match block.output_start_line {
            Some(output_start) => {
                let end_line = match block.status {
                    Some(CommandBlockStatus::Running) => cursor_line,
                    _ => block.end_line,
                };
                if end_line >= output_start {
                    extract_lines(term, output_start, end_line)
                } else {
                    String::new()
                }
            }
            None => String::new(),
        };
        TerminalCommandBlockContent {
            id: block.id,
            command: block.command.clone(),
            output,
            status: block.status.unwrap_or(CommandBlockStatus::FinishedUnknown),
            cwd: block.cwd.clone(),
        }
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn active_block_index(&self) -> Option<usize> {
        let id = self.active_block_id?;
        self.blocks.iter().position(|block| block.id == id)
    }

    fn active_block_mut(&mut self) -> Option<&mut CommandBlock> {
        let index = self.active_block_index()?;
        Some(&mut self.blocks[index])
    }

    fn close_or_discard_active_block<T>(&mut self, term: &Term<T>, next_prompt_line: i32) {
        let Some(index) = self.active_block_index() else {
            return;
        };
        self.active_block_id = None;
        let status = self.blocks[index].status;
        if status.is_none() {
            self.blocks.remove(index);
            return;
        }
        if matches!(status, Some(CommandBlockStatus::Running)) {
            let block = &mut self.blocks[index];
            block.status = Some(CommandBlockStatus::FinishedUnknown);
            let fallback_end = next_prompt_line.saturating_sub(1);
            let min_end = block.output_start_line.unwrap_or(block.prompt_line);
            block.end_line = fallback_end.max(block.prompt_line);
            if let Some(output_start) = block.output_start_line
                && block.end_line >= output_start
            {
                block.has_output = !extract_lines(term, output_start, block.end_line).is_empty();
            } else {
                block.end_line = min_end.max(block.prompt_line);
                block.has_output = false;
            }
        }
    }

    fn enforce_block_cap(&mut self) {
        if self.blocks.len() > MAX_COMMAND_BLOCKS {
            let overflow = self.blocks.len() - MAX_COMMAND_BLOCKS;
            self.blocks.drain(0..overflow);
        }
        self.sync_referenced_ids();
    }

    fn clear_all_blocks(&mut self) {
        self.blocks.clear();
        self.active_block_id = None;
        self.active_jump_block_id = None;
    }

    fn clear_viewport_blocks<T>(&mut self, term: &Term<T>) {
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        self.blocks
            .retain(|block| block.prompt_line < 0 && block.status.is_some());
        for block in &mut self.blocks {
            if !matches!(block.status, Some(CommandBlockStatus::Running)) && block.end_line >= 0 {
                block.end_line = (-1).max(block.prompt_line);
            }
        }
        self.sync_referenced_ids();
    }

    fn clear_scrollback_blocks<T>(&mut self, term: &Term<T>) {
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        self.blocks.retain(|block| block.prompt_line >= 0);
        self.sync_referenced_ids();
    }

    fn clear_blocks_below_cursor<T>(&mut self, term: &Term<T>) {
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        let cursor = term.grid().cursor.point;
        let erased_from = if cursor.column.0 == 0 {
            cursor.line.0
        } else {
            cursor.line.0 + 1
        };
        self.blocks.retain(|block| block.prompt_line < erased_from);
        for block in &mut self.blocks {
            if !matches!(block.status, Some(CommandBlockStatus::Running))
                && block.end_line >= erased_from
            {
                block.end_line = (erased_from - 1).max(block.prompt_line);
            }
        }
        self.sync_referenced_ids();
    }

    fn clear_blocks_above_cursor<T>(&mut self, term: &Term<T>) {
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        let cursor_line = term.grid().cursor.point.line.0;
        self.blocks
            .retain(|block| !(block.prompt_line >= 0 && block.prompt_line <= cursor_line));
        self.sync_referenced_ids();
    }

    fn sync_referenced_ids(&mut self) {
        if let Some(active_id) = self.active_block_id
            && !self.blocks.iter().any(|block| block.id == active_id)
        {
            self.active_block_id = None;
        }
        if let Some(focused_id) = self.active_jump_block_id
            && !self.blocks.iter().any(|block| block.id == focused_id)
        {
            self.active_jump_block_id = None;
        }
    }
}

fn command_end_and_output_start(
    command_start_line: i32,
    cursor_line: i32,
    cursor_col: usize,
    columns: usize,
) -> (Point, i32) {
    let last_col = columns.saturating_sub(1);
    if cursor_col == 0 && cursor_line > command_start_line {
        (
            Point::new(Line(cursor_line - 1), Column(last_col)),
            cursor_line,
        )
    } else if cursor_col == 0 {
        (Point::new(Line(cursor_line), Column(0)), cursor_line)
    } else {
        (
            Point::new(Line(cursor_line), Column(cursor_col.saturating_sub(1))),
            cursor_line + 1,
        )
    }
}

fn display_offset_for_prompt_line(
    prompt_line: i32,
    screen_lines: usize,
    current_offset: usize,
    history_size: usize,
) -> usize {
    let viewport_row = prompt_line + current_offset as i32;
    if viewport_row >= 0 && viewport_row < screen_lines as i32 {
        return current_offset;
    }
    if prompt_line < 0 {
        (prompt_line.unsigned_abs() as usize + 1).min(history_size)
    } else {
        0
    }
}

fn extract_lines<T>(term: &Term<T>, start_line: i32, end_line: i32) -> String {
    let columns = term.columns();
    if columns == 0 {
        return String::new();
    }
    let topmost = -(term.grid().history_size() as i32);
    let bottom = term.screen_lines() as i32 - 1;
    let start = start_line.max(topmost);
    let end = end_line.min(bottom);
    if start > end {
        return String::new();
    }
    extract_range(
        term,
        Point::new(Line(start), Column(0)),
        Point::new(Line(end), Column(columns - 1)),
    )
}

fn extract_range<T>(term: &Term<T>, start: Point, end: Point) -> String {
    let columns = term.columns();
    if columns == 0 {
        return String::new();
    }
    let topmost = -(term.grid().history_size() as i32);
    let bottom = term.screen_lines() as i32 - 1;
    let start = Point::new(
        Line(start.line.0.clamp(topmost, bottom)),
        Column(start.column.0.min(columns - 1)),
    );
    let end = Point::new(
        Line(end.line.0.clamp(topmost, bottom)),
        Column(end.column.0.min(columns - 1)),
    );
    if start > end {
        return String::new();
    }

    let raw = term.bounds_to_string(start, end);
    raw.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

#[derive(Clone, Copy, Debug)]
struct LogicalLineSpan {
    start_line: i32,
    end_line: i32,
}

#[derive(Clone, Copy, Debug)]
enum ReflowAnchorKind {
    CharOffset(usize),
    LogicalEnd,
}

#[derive(Clone, Copy, Debug)]
enum ReflowAnchor {
    BeforeTop(i32),
    Inside {
        logical_idx: usize,
        kind: ReflowAnchorKind,
    },
    AfterBottom(i32),
}

#[derive(Clone, Copy, Debug)]
struct BlockReflowAnchors {
    prompt: ReflowAnchor,
    command_start: Option<ReflowAnchor>,
    output_start: Option<ReflowAnchor>,
    end: ReflowAnchor,
}

pub(crate) struct ReflowSnapshot {
    blocks: Vec<BlockReflowAnchors>,
}

fn collect_logical_lines<T>(term: &Term<T>) -> Vec<LogicalLineSpan> {
    let cols = term.columns();
    let screen_lines = term.screen_lines();
    if cols == 0 || screen_lines == 0 {
        return Vec::new();
    }
    let topmost = -(term.grid().history_size() as i32);
    let bottom = screen_lines as i32 - 1;
    if topmost > bottom {
        return Vec::new();
    }

    let grid = term.grid();
    let last_col = Column(cols - 1);
    let mut spans = Vec::new();
    let mut span_start = topmost;

    for line in topmost..=bottom {
        let wraps = grid[Line(line)][last_col].flags.contains(Flags::WRAPLINE);
        if !wraps || line == bottom {
            spans.push(LogicalLineSpan {
                start_line: span_start,
                end_line: line,
            });
            span_start = line + 1;
        }
    }

    spans
}

fn find_span_index(spans: &[LogicalLineSpan], line: i32) -> Option<usize> {
    spans
        .binary_search_by(|span| {
            if line < span.start_line {
                std::cmp::Ordering::Greater
            } else if line > span.end_line {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .ok()
}

fn anchor_point(
    line: i32,
    col: usize,
    spans: &[LogicalLineSpan],
    cols: usize,
    topmost: i32,
    bottom: i32,
) -> ReflowAnchor {
    if line < topmost {
        return ReflowAnchor::BeforeTop(line - topmost);
    }
    if line > bottom {
        return ReflowAnchor::AfterBottom(line - bottom);
    }
    if let Some(logical_idx) = find_span_index(spans, line) {
        let row_offset = (line - spans[logical_idx].start_line).max(0) as usize;
        let char_offset = row_offset
            .saturating_mul(cols)
            .saturating_add(col.min(cols.saturating_sub(1)));
        ReflowAnchor::Inside {
            logical_idx,
            kind: ReflowAnchorKind::CharOffset(char_offset),
        }
    } else {
        ReflowAnchor::AfterBottom(0)
    }
}

fn anchor_end(
    line: i32,
    spans: &[LogicalLineSpan],
    cols: usize,
    topmost: i32,
    bottom: i32,
) -> ReflowAnchor {
    if line < topmost {
        return ReflowAnchor::BeforeTop(line - topmost);
    }
    if line > bottom {
        return ReflowAnchor::AfterBottom(line - bottom);
    }
    if let Some(logical_idx) = find_span_index(spans, line) {
        let span = spans[logical_idx];
        let kind = if line == span.end_line {
            ReflowAnchorKind::LogicalEnd
        } else {
            let row_offset = (line - span.start_line).max(0) as usize;
            ReflowAnchorKind::CharOffset(
                row_offset
                    .saturating_mul(cols)
                    .saturating_add(cols.saturating_sub(1)),
            )
        };
        ReflowAnchor::Inside { logical_idx, kind }
    } else {
        ReflowAnchor::AfterBottom(0)
    }
}

fn resolve_anchor(
    anchor: ReflowAnchor,
    spans: &[LogicalLineSpan],
    cols: usize,
    topmost: i32,
    bottom: i32,
) -> (i32, usize) {
    match anchor {
        ReflowAnchor::BeforeTop(delta) => (topmost.saturating_add(delta), 0),
        ReflowAnchor::AfterBottom(delta) => (bottom.saturating_add(delta), 0),
        ReflowAnchor::Inside { logical_idx, kind } => {
            let Some(span) = spans.get(logical_idx).or_else(|| spans.last()) else {
                return (bottom, 0);
            };
            match kind {
                ReflowAnchorKind::LogicalEnd => (span.end_line, cols.saturating_sub(1)),
                ReflowAnchorKind::CharOffset(offset) => {
                    let row_offset = i32::try_from(offset / cols).unwrap_or(i32::MAX);
                    let line = span
                        .start_line
                        .saturating_add(row_offset)
                        .min(span.end_line);
                    let col = (offset % cols).min(cols.saturating_sub(1));
                    (line, col)
                }
            }
        }
    }
}

pub(crate) fn height_resize_line_shift(
    old_lines: usize,
    new_lines: usize,
    cursor_line: i32,
    history_size: usize,
) -> i32 {
    if new_lines > old_lines {
        let pulled = history_size.min(new_lines - old_lines);
        i32::try_from(pulled).unwrap_or(i32::MAX)
    } else if new_lines < old_lines {
        let scrolled = (cursor_line.max(0) as usize + 1).saturating_sub(new_lines);
        -i32::try_from(scrolled).unwrap_or(i32::MAX)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc133_scanner_splits_across_arbitrary_chunks() {
        let mut scanner = Osc133Scanner::default();
        let stream = concat!(
            "prompt$ ",
            "\x1b]133;B\x07",
            "echo hi\r\n",
            "\x1b]133;C\x1b\\",
            "hi\r\n",
            "\x1b]133;D;0;aid=42\x07",
            "\x1b]133;P;k=i\x07",
        );

        let mut marks = Vec::new();
        let mut rebuilt = Vec::new();
        for chunk in stream.as_bytes().chunks(3) {
            scanner.feed_segments(chunk, |slice, event| {
                rebuilt.extend_from_slice(slice);
                if let Some(TerminalControlEvent::Osc133(mark)) = event {
                    marks.push(mark);
                }
            });
        }

        assert_eq!(rebuilt, stream.as_bytes());
        assert_eq!(
            marks,
            vec![
                Osc133Mark::CommandStart,
                Osc133Mark::CommandExecuted,
                Osc133Mark::CommandFinished { exit_code: Some(0) },
            ]
        );
    }

    #[test]
    fn osc133_scanner_ignores_other_osc_codes_like_osc7_and_osc1337() {
        let mut scanner = Osc133Scanner::default();
        let stream =
            b"\x1b]7;file://host/tmp\x07\x1b]1337;SetUserVar=foo=bar\x07\x1b]133;D;127\x1b\\";
        let mut marks = Vec::new();
        scanner.feed_segments(stream, |_, event| {
            if let Some(TerminalControlEvent::Osc133(mark)) = event {
                marks.push(mark);
            }
        });
        assert_eq!(
            marks,
            vec![Osc133Mark::CommandFinished {
                exit_code: Some(127)
            }]
        );
    }

    #[test]
    fn scanner_tracks_csi_clear_and_esc_reset_across_chunks() {
        let stream = concat!(
            "before\r\n",
            "\x1b[H\x1b[2J\x1b[3J",
            "\x1b[J",
            "\x1b[0J",
            "\x1b[1J",
            "\x1b[?2J",
            "\x1b(c",
            "mid",
            "\x1bc",
            "after",
        );

        for chunk_size in [1, 2, 3, stream.len()] {
            let mut scanner = Osc133Scanner::default();
            let mut events = Vec::new();
            let mut rebuilt = Vec::new();

            for chunk in stream.as_bytes().chunks(chunk_size) {
                scanner.feed_segments(chunk, |slice, event| {
                    rebuilt.extend_from_slice(slice);
                    if let Some(event) = event {
                        events.push((String::from_utf8_lossy(slice).into_owned(), event));
                    }
                });
            }

            assert_eq!(rebuilt, stream.as_bytes());
            assert_eq!(
                events.iter().map(|(_, e)| *e).collect::<Vec<_>>(),
                vec![
                    TerminalControlEvent::ClearAll,
                    TerminalControlEvent::ClearSaved,
                    TerminalControlEvent::ClearBelow,
                    TerminalControlEvent::ClearBelow,
                    TerminalControlEvent::ClearAbove,
                    TerminalControlEvent::Reset,
                ]
            );
            if chunk_size == stream.len() {
                assert_eq!(
                    events
                        .iter()
                        .map(|(slice, _)| slice.as_str())
                        .collect::<Vec<_>>(),
                    vec![
                        "\x1b[2J", "\x1b[3J", "\x1b[J", "\x1b[0J", "\x1b[1J", "\x1bc",
                    ]
                );
            }
        }
    }

    #[test]
    fn height_resize_line_shift_matches_alacritty_grid_motion() {
        assert_eq!(height_resize_line_shift(8, 12, 7, 10), 4);
        assert_eq!(height_resize_line_shift(8, 12, 7, 2), 2);
        assert_eq!(height_resize_line_shift(8, 5, 7, 10), -3);
        assert_eq!(height_resize_line_shift(8, 5, 2, 10), 0);
        assert_eq!(height_resize_line_shift(8, 8, 4, 10), 0);
    }
}
