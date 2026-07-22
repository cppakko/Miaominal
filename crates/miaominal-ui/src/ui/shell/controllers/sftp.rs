use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    path::{Component, Path, PathBuf},
    pin::Pin,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context as _, Result};
use futures::StreamExt;
use gpui::{
    AppContext as _, Bounds, Context, Entity, EventEmitter, Pixels, Point, ScrollHandle,
    SharedString, Subscription, Window, px,
};
use gpui_component::{
    WindowExt as _,
    input::{InputEvent, InputState},
    scroll::ScrollbarHandle as _,
    table::{TableEvent, TableState},
};
use miaominal_core::profile::SessionProfile;
use miaominal_services::{PlannedSftpDownload, SftpService};

use super::AppCommand;
use crate::ui::{
    i18n,
    shell::{
        DialogOverlaySnapshot, SESSION_SFTP_PROGRESS_DEFAULT_FLEX, SessionQueryPort,
        SftpBrowserSelectionModifiers, SftpBrowserSide, SftpBrowserTableDelegate,
        SftpBrowserTableRow, TabId, TabKindTag, TabState, ValidationNotificationKind,
        error_notification, new_input_state, set_input_placeholder, set_input_value,
        success_notification, support::CONTAINER_TRANSITION_DURATION, validation_notification,
    },
};
use miaominal_sftp::{
    SftpCommandSender, SftpDirectoryRequestId, SftpEntry, SftpEvent, SftpEventReceiver,
    SftpProgressReceiver, SftpTransferChild, SftpTransferChildState, SftpTransferChildUpdate,
    SftpTransferProgress, TransferChildId, TransferDirection, TransferId,
};
use notify::{EventKind, RecursiveMode, Watcher};
use rfd::AsyncFileDialog;

const SFTP_TRANSFER_CHILD_HISTORY_LIMIT: usize = 500;
const SFTP_PROGRESS_REFRESH_INTERVAL: Duration = Duration::from_millis(100);
const SFTP_DIRECTORY_REFRESH_DEBOUNCE: Duration = Duration::from_millis(150);
const SFTP_DRAG_SELECTION_THRESHOLD: f32 = 4.0;
const SFTP_DRAG_AUTO_SCROLL_EDGE_ZONE: f32 = 72.0;
const SFTP_DRAG_AUTO_SCROLL_MIN_STEP: f32 = 0.75;
const SFTP_DRAG_AUTO_SCROLL_MAX_STEP: f32 = 14.0;
const SFTP_DRAG_AUTO_SCROLL_MAX_RATIO: f32 = 2.25;
const SFTP_DRAG_AUTO_SCROLL_INTERVAL: Duration = Duration::from_millis(16);

struct PreparedSftpDownloads {
    downloads: Vec<PlannedSftpDownload>,
    overwrite_confirmed: bool,
}

fn prepare_single_sftp_file_download(
    entry: &SftpEntry,
    local_path: PathBuf,
) -> PreparedSftpDownloads {
    PreparedSftpDownloads {
        downloads: vec![PlannedSftpDownload {
            remote_path: entry.path.clone(),
            local_path,
        }],
        overwrite_confirmed: true,
    }
}

fn should_use_single_file_save_dialog(selected_entries: &[SftpEntry]) -> bool {
    selected_entries.len() == 1 && selected_entries[0].kind == miaominal_sftp::SftpEntryKind::File
}

fn choose_sftp_download_destination(
    selected_entries: Vec<SftpEntry>,
    initial_directory: &Path,
    window: &Window,
) -> Pin<Box<dyn Future<Output = Option<PreparedSftpDownloads>> + 'static>> {
    if should_use_single_file_save_dialog(&selected_entries) {
        let entry = selected_entries[0].clone();
        let mut dialog = AsyncFileDialog::new()
            .set_parent(window)
            .set_title(i18n::string("sftp.dialogs.download_file_title"))
            .set_file_name(entry.filename.clone());
        if initial_directory.is_dir() {
            dialog = dialog.set_directory(initial_directory);
        }
        return Box::pin(async move {
            let local_path = dialog.save_file().await?.path().to_path_buf();
            Some(prepare_single_sftp_file_download(&entry, local_path))
        });
    }

    let mut dialog = AsyncFileDialog::new()
        .set_parent(window)
        .set_title(i18n::string("sftp.dialogs.download_folder_title"));
    if initial_directory.is_dir() {
        dialog = dialog.set_directory(initial_directory);
    }
    Box::pin(async move {
        let local_base = dialog.pick_folder().await?.path().to_path_buf();
        Some(PreparedSftpDownloads {
            downloads: SftpService::plan_downloads(selected_entries, &local_base),
            overwrite_confirmed: false,
        })
    })
}

fn normalize_remote_delete_entries(entries: Vec<(String, bool)>) -> Vec<(String, bool)> {
    let mut seen_paths = HashSet::new();
    let unique_entries = entries
        .into_iter()
        .filter(|(path, _)| seen_paths.insert(path.clone()))
        .collect::<Vec<_>>();
    let directory_paths = unique_entries
        .iter()
        .filter(|(_, is_directory)| *is_directory)
        .map(|(path, _)| remote_path_without_trailing_separator(path))
        .collect::<HashSet<_>>();

    unique_entries
        .into_iter()
        .filter(|(path, _)| !remote_path_has_selected_directory_ancestor(path, &directory_paths))
        .collect()
}

fn remote_path_has_selected_directory_ancestor(
    path: &str,
    directory_paths: &HashSet<String>,
) -> bool {
    let normalized = remote_path_without_trailing_separator(path);
    if remote_path_contains_unsafe_component(&normalized) {
        return false;
    }

    let mut current = normalized.as_str();
    while let Some(parent) = remote_parent_path_component(current) {
        if directory_paths.contains(parent) {
            return true;
        }
        current = parent;
    }
    false
}

fn remote_path_without_trailing_separator(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() && path.starts_with('/') {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn remote_path_contains_unsafe_component(path: &str) -> bool {
    path.split('/')
        .any(|component| matches!(component, "." | ".."))
}

fn remote_parent_path_component(path: &str) -> Option<&str> {
    let path = path.trim_end_matches('/');
    if path.is_empty() || path == "/" {
        return None;
    }

    match path.rfind('/') {
        Some(0) => Some("/"),
        Some(index) => Some(path[..index].trim_end_matches('/')),
        None => None,
    }
}

#[derive(Debug)]
struct LocalDeleteResult {
    deleted_count: usize,
    first_error: Option<(PathBuf, String)>,
}

fn remove_local_entry(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn delete_local_entries(entries: Vec<PathBuf>) -> LocalDeleteResult {
    let mut deleted_count = 0;
    let mut first_error = None;

    for path in entries {
        match remove_local_entry(&path) {
            Ok(()) => deleted_count += 1,
            Err(error) if first_error.is_none() => {
                first_error = Some((path, error.to_string()));
            }
            Err(_) => {}
        }
    }

    LocalDeleteResult {
        deleted_count,
        first_error,
    }
}

fn create_local_directory(parent: &Path, name: &str) -> std::io::Result<PathBuf> {
    let path = parent.join(name);
    std::fs::create_dir(&path)?;
    Ok(path)
}

fn is_valid_local_directory_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SftpProgressCenterTransitionPhase {
    Entering,
    Exiting,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct SftpProgressCenterTransition {
    pub(in crate::ui::shell) phase: SftpProgressCenterTransitionPhase,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SessionSftpProgressCenterDragState {
    pub(in crate::ui::shell) initial_pointer: f32,
    pub(in crate::ui::shell) initial_flex: f32,
    pub(in crate::ui::shell) container_height: Pixels,
}

struct SftpProgressLayoutState {
    session_visible: bool,
    session_transition: Option<SftpProgressCenterTransition>,
    scroll_handle: ScrollHandle,
    session_flex: f32,
    session_drag: Option<SessionSftpProgressCenterDragState>,
}

impl Default for SftpProgressLayoutState {
    fn default() -> Self {
        Self {
            session_visible: false,
            session_transition: None,
            scroll_handle: ScrollHandle::new(),
            session_flex: SESSION_SFTP_PROGRESS_DEFAULT_FLEX,
            session_drag: None,
        }
    }
}

fn update_progress_center_state(
    current_visible: &mut bool,
    transition: &mut Option<SftpProgressCenterTransition>,
    visible: bool,
    started_at: Instant,
) -> bool {
    let phase = if visible {
        SftpProgressCenterTransitionPhase::Entering
    } else {
        SftpProgressCenterTransitionPhase::Exiting
    };
    if *current_visible == visible
        && transition
            .as_ref()
            .is_none_or(|transition| transition.phase == phase)
    {
        return false;
    }

    *current_visible = visible;
    *transition = Some(SftpProgressCenterTransition {
        phase,
        started_at,
        duration: CONTAINER_TRANSITION_DURATION,
    });
    true
}

fn progress_center_render_visibility(
    visible: bool,
    transition: &mut Option<SftpProgressCenterTransition>,
    window: &mut Window,
) -> Option<f32> {
    let Some(current_transition) = *transition else {
        return visible.then_some(1.0);
    };

    let duration_seconds = current_transition.duration.as_secs_f32();
    if duration_seconds <= f32::EPSILON {
        *transition = None;
        return visible.then_some(1.0);
    }

    let elapsed = Instant::now().saturating_duration_since(current_transition.started_at);
    let progress = (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0);
    let eased = progress * progress * (3.0 - 2.0 * progress);

    if progress >= 1.0 {
        *transition = None;
        return visible.then_some(1.0);
    }

    window.request_animation_frame();

    Some(match current_transition.phase {
        SftpProgressCenterTransitionPhase::Entering => eased,
        SftpProgressCenterTransitionPhase::Exiting => 1.0 - eased,
    })
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct LocalSftpEntry {
    pub(in crate::ui::shell) filename: String,
    pub(in crate::ui::shell) path: PathBuf,
    pub(in crate::ui::shell) is_directory: bool,
    pub(in crate::ui::shell) size: Option<u64>,
    pub(in crate::ui::shell) modified: Option<SystemTime>,
    pub(in crate::ui::shell) attributes: Option<String>,
    pub(in crate::ui::shell) owner: Option<String>,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) enum SftpTransferStatus {
    Queued,
    Running,
    Paused,
    Done,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) enum SftpTransferChildStatus {
    Running,
    Paused,
    Done,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SftpTransferChildRow {
    pub(in crate::ui::shell) child_id: TransferChildId,
    pub(in crate::ui::shell) relative_path: String,
    pub(in crate::ui::shell) bytes_complete: u64,
    pub(in crate::ui::shell) bytes_total: Option<u64>,
    pub(in crate::ui::shell) status: SftpTransferChildStatus,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SftpTransferRow {
    pub(in crate::ui::shell) transfer_id: TransferId,
    pub(in crate::ui::shell) direction: TransferDirection,
    pub(in crate::ui::shell) source: PathBuf,
    pub(in crate::ui::shell) destination: String,
    pub(in crate::ui::shell) bytes_complete: u64,
    pub(in crate::ui::shell) bytes_total: Option<u64>,
    pub(in crate::ui::shell) status: SftpTransferStatus,
    pub(in crate::ui::shell) bytes_per_second: Option<u64>,
    pub(in crate::ui::shell) last_progress_at: Option<Instant>,
    pub(in crate::ui::shell) last_bytes_complete: u64,
    pub(in crate::ui::shell) is_directory: bool,
    pub(in crate::ui::shell) expanded: bool,
    pub(in crate::ui::shell) children: VecDeque<SftpTransferChildRow>,
    pub(in crate::ui::shell) child_count: u64,
}

impl SftpTransferRow {
    pub(in crate::ui::shell) fn push_child(&mut self, child: SftpTransferChild) {
        self.is_directory = true;
        self.child_count = self.child_count.saturating_add(1);
        if self.children.len() == SFTP_TRANSFER_CHILD_HISTORY_LIMIT {
            self.children.pop_front();
        }
        self.children.push_back(SftpTransferChildRow {
            child_id: child.child_id,
            relative_path: child.relative_path,
            bytes_complete: 0,
            bytes_total: child.bytes_total,
            status: SftpTransferChildStatus::Running,
        });
    }

    pub(in crate::ui::shell) fn omitted_child_count(&self) -> u64 {
        self.child_count.saturating_sub(self.children.len() as u64)
    }

    pub(in crate::ui::shell) fn apply_child_update(&mut self, update: SftpTransferChildUpdate) {
        let Some(child) = self
            .children
            .iter_mut()
            .find(|child| child.child_id == update.child_id)
        else {
            return;
        };
        if matches!(update.state, SftpTransferChildState::Running)
            && matches!(
                child.status,
                SftpTransferChildStatus::Done
                    | SftpTransferChildStatus::Cancelled
                    | SftpTransferChildStatus::Failed(_)
            )
        {
            return;
        }
        child.bytes_complete = update.bytes_complete;
        child.status = match update.state {
            SftpTransferChildState::Running => SftpTransferChildStatus::Running,
            SftpTransferChildState::Done => {
                if let Some(total) = child.bytes_total {
                    child.bytes_complete = total;
                }
                SftpTransferChildStatus::Done
            }
            SftpTransferChildState::Cancelled => SftpTransferChildStatus::Cancelled,
            SftpTransferChildState::Failed(message) => SftpTransferChildStatus::Failed(message),
        };
    }

    pub(in crate::ui::shell) fn pause_active_child(&mut self) {
        if let Some(child) = self
            .children
            .iter_mut()
            .find(|child| matches!(child.status, SftpTransferChildStatus::Running))
        {
            child.status = SftpTransferChildStatus::Paused;
        }
    }

    pub(in crate::ui::shell) fn resume_active_child(&mut self) {
        if let Some(child) = self
            .children
            .iter_mut()
            .find(|child| matches!(child.status, SftpTransferChildStatus::Paused))
        {
            child.status = SftpTransferChildStatus::Running;
        }
    }

    pub(in crate::ui::shell) fn cancel_unfinished_children(&mut self) {
        for child in &mut self.children {
            if matches!(
                child.status,
                SftpTransferChildStatus::Running | SftpTransferChildStatus::Paused
            ) {
                child.status = SftpTransferChildStatus::Cancelled;
            }
        }
    }

    pub(in crate::ui::shell) fn fail_unfinished_children(&mut self, message: &str) {
        if !self
            .children
            .iter()
            .any(|child| matches!(child.status, SftpTransferChildStatus::Failed(_)))
            && let Some(child) = self.children.iter_mut().find(|child| {
                matches!(
                    child.status,
                    SftpTransferChildStatus::Running | SftpTransferChildStatus::Paused
                )
            })
        {
            child.status = SftpTransferChildStatus::Failed(message.to_string());
        }
    }

    pub(in crate::ui::shell) fn apply_progress(&mut self, progress: SftpTransferProgress) {
        if matches!(
            self.status,
            SftpTransferStatus::Done
                | SftpTransferStatus::Cancelled
                | SftpTransferStatus::Failed(_)
        ) {
            return;
        }
        if progress.bytes_complete < self.bytes_complete {
            return;
        }

        let is_paused = matches!(self.status, SftpTransferStatus::Paused);
        let now = Instant::now();
        if let Some(sample_at) = self.last_progress_at {
            let elapsed = now.duration_since(sample_at).as_secs_f64();
            if elapsed >= 0.5 {
                let delta = progress
                    .bytes_complete
                    .saturating_sub(self.last_bytes_complete);
                self.bytes_per_second = Some((delta as f64 / elapsed) as u64);
                self.last_progress_at = Some(now);
                self.last_bytes_complete = progress.bytes_complete;
            }
        } else {
            self.last_progress_at = Some(now);
            self.last_bytes_complete = progress.bytes_complete;
        }
        self.bytes_complete = progress.bytes_complete;
        self.bytes_total = progress.bytes_total;
        if !is_paused {
            self.status = SftpTransferStatus::Running;
        }
        if let Some(child) = progress.child {
            self.apply_child_update(child);
            if is_paused {
                self.pause_active_child();
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct SftpDragSelectionState {
    pub(in crate::ui::shell) start: Point<Pixels>,
    pub(in crate::ui::shell) current: Point<Pixels>,
    pub(in crate::ui::shell) last_row_range: Option<(usize, usize)>,
}

impl SftpDragSelectionState {
    pub(in crate::ui::shell) fn new(start: Point<Pixels>) -> Self {
        Self {
            start,
            current: start,
            last_row_range: None,
        }
    }

    pub(in crate::ui::shell) fn update(&mut self, current: Point<Pixels>) {
        self.current = current;
    }

    pub(in crate::ui::shell) fn bounds(&self) -> Bounds<Pixels> {
        let left = if self.start.x <= self.current.x {
            self.start.x
        } else {
            self.current.x
        };
        let top = if self.start.y <= self.current.y {
            self.start.y
        } else {
            self.current.y
        };
        let right = if self.start.x >= self.current.x {
            self.start.x
        } else {
            self.current.x
        };
        let bottom = if self.start.y >= self.current.y {
            self.start.y
        } else {
            self.current.y
        };

        Bounds::from_corners(Point::new(left, top), Point::new(right, bottom))
    }

    pub(in crate::ui::shell) fn exceeds_threshold(&self, threshold: Pixels) -> bool {
        let bounds = self.bounds();
        bounds.size.width >= threshold || bounds.size.height >= threshold
    }

    pub(in crate::ui::shell) fn set_last_row_range(
        &mut self,
        row_range: Option<(usize, usize)>,
    ) -> bool {
        if self.last_row_range == row_range {
            return false;
        }

        self.last_row_range = row_range;
        true
    }
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct SftpDragSelectionContext {
    pub(in crate::ui::shell) side: SftpBrowserSide,
    pub(in crate::ui::shell) tab_id: TabId,
    pub(in crate::ui::shell) last_position: Point<Pixels>,
    pub(in crate::ui::shell) panel_bounds: Bounds<Pixels>,
    pub(in crate::ui::shell) row_height: Pixels,
    pub(in crate::ui::shell) anchor_content_y: f32,
    pub(in crate::ui::shell) generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum SftpSplitDivider {
    BrowserPanels,
    ProgressCenter,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SftpSplitDragState {
    pub(in crate::ui::shell) divider: SftpSplitDivider,
    pub(in crate::ui::shell) initial_pointer: f32,
    pub(in crate::ui::shell) initial_flex_a: f32,
    pub(in crate::ui::shell) container_size: f32,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SftpLayoutState {
    pub(in crate::ui::shell) local_panel_flex: Option<f32>,
    pub(in crate::ui::shell) browser_area_flex: Option<f32>,
    pub(in crate::ui::shell) progress_center_visible: bool,
    pub(in crate::ui::shell) progress_center_transition: Option<SftpProgressCenterTransition>,
    pub(in crate::ui::shell) browser_container_width: Pixels,
    pub(in crate::ui::shell) page_container_height: Pixels,
    pub(in crate::ui::shell) drag: Option<SftpSplitDragState>,
}

impl Default for SftpLayoutState {
    fn default() -> Self {
        Self {
            local_panel_flex: None,
            browser_area_flex: None,
            progress_center_visible: false,
            progress_center_transition: None,
            browser_container_width: px(0.0),
            page_container_height: px(0.0),
            drag: None,
        }
    }
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SftpTabState {
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) commands: Option<SftpCommandSender>,
    pub(in crate::ui::shell) local_path: PathBuf,
    pub(in crate::ui::shell) local_entries: Vec<LocalSftpEntry>,
    pub(in crate::ui::shell) selected_local_path: Option<PathBuf>,
    pub(in crate::ui::shell) selected_local_paths: Vec<PathBuf>,
    pub(in crate::ui::shell) local_selection_anchor: Option<PathBuf>,
    pub(in crate::ui::shell) remote_path: String,
    pub(in crate::ui::shell) requested_remote_path: Option<String>,
    pub(in crate::ui::shell) remote_directory_request_id: Option<SftpDirectoryRequestId>,
    pub(in crate::ui::shell) remote_entries: Vec<SftpEntry>,
    pub(in crate::ui::shell) selected_remote_path: Option<String>,
    pub(in crate::ui::shell) selected_remote_paths: Vec<String>,
    pub(in crate::ui::shell) remote_selection_anchor: Option<String>,
    pub(in crate::ui::shell) transfers: Vec<SftpTransferRow>,
    pub(in crate::ui::shell) last_status: String,
    pub(in crate::ui::shell) last_error: Option<String>,
    pub(in crate::ui::shell) loading_remote: bool,
    pub(in crate::ui::shell) local_drag_candidate: Option<Point<Pixels>>,
    pub(in crate::ui::shell) remote_drag_candidate: Option<Point<Pixels>>,
    pub(in crate::ui::shell) local_drag_selection: Option<SftpDragSelectionState>,
    pub(in crate::ui::shell) remote_drag_selection: Option<SftpDragSelectionState>,
    pub(in crate::ui::shell) drag_selection_context: Option<SftpDragSelectionContext>,
    pub(in crate::ui::shell) drag_selection_generation: u64,
    pub(in crate::ui::shell) suppress_local_clear_click: bool,
    pub(in crate::ui::shell) suppress_remote_clear_click: bool,
    pub(in crate::ui::shell) pending_local_refresh_paths: HashSet<PathBuf>,
    pub(in crate::ui::shell) pending_remote_refresh_paths: HashSet<String>,
    pub(in crate::ui::shell) directory_refresh_generation: u64,
    pub(in crate::ui::shell) local_scan_generation: u64,
    pub(in crate::ui::shell) layout: SftpLayoutState,
}

impl SftpTabState {
    pub(in crate::ui::shell) fn new(profile: &SessionProfile) -> Self {
        Self {
            profile_id: profile.id.clone(),
            commands: None,
            local_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            local_entries: Vec::new(),
            selected_local_path: None,
            selected_local_paths: Vec::new(),
            local_selection_anchor: None,
            remote_path: ".".into(),
            requested_remote_path: None,
            remote_directory_request_id: None,
            remote_entries: Vec::new(),
            selected_remote_path: None,
            selected_remote_paths: Vec::new(),
            remote_selection_anchor: None,
            transfers: Vec::new(),
            last_status: i18n::string("tabs.initial.sftp_starting_worker"),
            last_error: None,
            loading_remote: true,
            local_drag_candidate: None,
            remote_drag_candidate: None,
            local_drag_selection: None,
            remote_drag_selection: None,
            drag_selection_context: None,
            drag_selection_generation: 0,
            suppress_local_clear_click: false,
            suppress_remote_clear_click: false,
            pending_local_refresh_paths: HashSet::new(),
            pending_remote_refresh_paths: HashSet::new(),
            directory_refresh_generation: 0,
            local_scan_generation: 0,
            layout: SftpLayoutState::default(),
        }
    }
}

impl TabState {
    pub(in crate::ui::shell) fn new_sftp(id: TabId, profile: &SessionProfile) -> Self {
        Self::new(
            id,
            profile.connection_label(),
            i18n::string("tabs.initial.sftp_connecting"),
            TabKindTag::Sftp,
            crate::ui::shell::workspace::TabPlacement::TopLevel,
        )
    }
}

#[derive(Default)]
struct SftpTabStore {
    tabs: HashMap<TabId, SftpTabState>,
}

impl SftpTabStore {
    fn insert(&mut self, tab_id: TabId, tab: SftpTabState) {
        assert!(
            self.tabs.insert(tab_id, tab).is_none(),
            "duplicate SFTP tab payload for {tab_id}"
        );
    }

    fn remove(&mut self, tab_id: TabId) -> Option<SftpTabState> {
        self.tabs.remove(&tab_id)
    }
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) enum SftpPromptKind {
    CreateRemoteDirectory {
        parent: String,
    },
    CreateLocalDirectory {
        parent: PathBuf,
    },
    ConfirmOverwrite {
        conflict_count: usize,
        pending_uploads: Vec<(PathBuf, String)>,
        pending_downloads: Vec<(String, PathBuf)>,
    },
    ConfirmDelete {
        entries: Vec<(String, bool)>,
        refresh_path: String,
    },
    ConfirmDeleteLocal {
        entries: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SftpPromptState {
    pub(in crate::ui::shell) kind: SftpPromptKind,
}

#[derive(Debug, Clone)]
enum InlineRenameState {
    Local { from: PathBuf, parent: PathBuf },
    Remote { from: String, parent: String },
}

struct SftpEditSession {
    pub(in crate::ui::shell) temp_path: PathBuf,
    pub(in crate::ui::shell) _watcher: notify::RecommendedWatcher,
    pub(in crate::ui::shell) debounce_task: Option<gpui::Task<()>>,
    pub(in crate::ui::shell) _watch_task: gpui::Task<()>,
}

pub(in crate::ui::shell) struct SftpBrowserForms {
    pub(in crate::ui::shell) local_path_input: Entity<InputState>,
    pub(in crate::ui::shell) remote_path_input: Entity<InputState>,
    pub(in crate::ui::shell) local_path_editing: bool,
    pub(in crate::ui::shell) remote_path_editing: bool,
    pub(in crate::ui::shell) remote_path_submit_pending: bool,
    pub(in crate::ui::shell) local_table: Entity<TableState<SftpBrowserTableDelegate>>,
    pub(in crate::ui::shell) remote_table: Entity<TableState<SftpBrowserTableDelegate>>,
    pub(in crate::ui::shell) prompt_input: Entity<InputState>,
    pub(in crate::ui::shell) inline_rename_input: Entity<InputState>,
}

pub(in crate::ui::shell) struct SftpControllerArgs {
    pub(in crate::ui::shell) service: SftpService,
    pub(in crate::ui::shell) local_hidden_columns: Vec<usize>,
    pub(in crate::ui::shell) remote_hidden_columns: Vec<usize>,
}

enum SftpEditSessionStartError {
    CreateWatcher(String),
    WatchDirectory(String),
}

pub(in crate::ui::shell) struct SftpController {
    service: SftpService,
    session_query: SessionQueryPort,
    forms: RefCell<SftpBrowserForms>,
    tabs: RefCell<SftpTabStore>,
    interactions: RefCell<HashMap<TabId, SftpInteractionState>>,
    progress_layout: RefCell<SftpProgressLayoutState>,
    download_destination_prompt_tab: Cell<Option<TabId>>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
struct SftpInteractionState {
    prompt: Option<SftpPromptState>,
    inline_rename: Option<InlineRenameState>,
    edit_pending_downloads: HashMap<TransferId, String>,
    edit_sessions: HashMap<String, SftpEditSession>,
    event_task: Option<gpui::Task<()>>,
    progress_task: Option<gpui::Task<()>>,
}

impl SftpController {
    fn focus_path_input_in_active_window(&self, side: SftpBrowserSide, cx: &mut Context<Self>) {
        let input = match side {
            SftpBrowserSide::Local => self.local_path_input(),
            SftpBrowserSide::Remote => self.remote_path_input(),
        };
        let Some(window_handle) = cx.active_window() else {
            return;
        };
        let _ = window_handle.update(cx, move |_, window, cx| {
            input.update(cx, |input, cx| input.focus(window, cx));
        });
    }

    fn sync_visible_path_input_in_active_window(
        &self,
        tab_id: TabId,
        side: SftpBrowserSide,
        cx: &mut Context<Self>,
    ) {
        if self.browser_tab_id(cx) != Some(tab_id) {
            return;
        }
        let Some(value) = self.tab(tab_id).map(|tab| match side {
            SftpBrowserSide::Local => Self::display_local_path(&tab.local_path),
            SftpBrowserSide::Remote => SharedString::from(tab.remote_path.clone()),
        }) else {
            return;
        };
        let input = match side {
            SftpBrowserSide::Local => self.local_path_input(),
            SftpBrowserSide::Remote => self.remote_path_input(),
        };
        let Some(window_handle) = cx.active_window() else {
            return;
        };
        let _ = window_handle.update(cx, move |_, window, cx| {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        });
    }

    fn notify_validation_failure(
        &self,
        kind: ValidationNotificationKind,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let notification = validation_notification(kind, message);
        if let Some(window_handle) = cx.active_window() {
            let _ = window_handle.update(cx, move |_, window, cx| {
                window.push_notification(notification, cx);
            });
        }
        cx.notify();
    }

    fn notify_download_completed(&self, filename: String, cx: &mut Context<Self>) {
        let title = i18n::string("sftp.notifications.download_complete_title");
        let body = i18n::string_args(
            "sftp.notifications.download_complete_message",
            &[("filename", &filename)],
        );
        let notification = success_notification(title, body);
        if let Some(window_handle) = cx.active_window() {
            let _ = window_handle.update(cx, move |_, window, cx| {
                window.push_notification(notification, cx);
            });
        }
        cx.notify();
    }

    fn notify_transfer_failed(&self, message: String, cx: &mut Context<Self>) {
        let title = i18n::string("sftp.notifications.transfer_failed_title");
        let notification = error_notification(title, message);
        if let Some(window_handle) = cx.active_window() {
            let _ = window_handle.update(cx, move |_, window, cx| {
                window.push_notification(notification, cx);
            });
        }
        cx.notify();
    }

    fn build_forms(
        local_hidden_columns: Vec<usize>,
        remote_hidden_columns: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SftpBrowserForms {
        let local_path_input = new_input_state(
            i18n::string("placeholders.sftp.local_path"),
            "",
            false,
            window,
            cx,
        );
        let remote_path_input = new_input_state(
            i18n::string("placeholders.sftp.remote_path"),
            ".",
            false,
            window,
            cx,
        );
        let local_table = cx.new(|cx| {
            let mut table = TableState::new(
                SftpBrowserTableDelegate::new(SftpBrowserSide::Local),
                window,
                cx,
            )
            .sortable(true)
            .col_movable(false)
            .col_resizable(true)
            .col_selectable(false)
            .row_selectable(false);
            table.col_fixed = false;
            table
        });
        let remote_table = cx.new(|cx| {
            let mut table = TableState::new(
                SftpBrowserTableDelegate::new(SftpBrowserSide::Remote),
                window,
                cx,
            )
            .sortable(true)
            .col_movable(false)
            .col_resizable(true)
            .col_selectable(false)
            .row_selectable(false);
            table.col_fixed = false;
            table
        });
        local_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .set_hidden_columns(local_hidden_columns);
            table.refresh(cx);
        });
        remote_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .set_hidden_columns(remote_hidden_columns);
            table.refresh(cx);
        });

        SftpBrowserForms {
            local_path_input,
            remote_path_input,
            local_path_editing: false,
            remote_path_editing: false,
            remote_path_submit_pending: false,
            local_table,
            remote_table,
            prompt_input: new_input_state(
                i18n::string("placeholders.sftp.remote_path"),
                "",
                false,
                window,
                cx,
            ),
            inline_rename_input: new_input_state(
                i18n::string("placeholders.sftp.new_name"),
                "",
                false,
                window,
                cx,
            ),
        }
    }

    pub(in crate::ui::shell) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forms = self.forms.borrow();
        for (input, key) in [
            (&forms.local_path_input, "placeholders.sftp.local_path"),
            (&forms.remote_path_input, "placeholders.sftp.remote_path"),
            (&forms.prompt_input, "placeholders.sftp.remote_path"),
            (&forms.inline_rename_input, "placeholders.sftp.new_name"),
        ] {
            set_input_placeholder(input, i18n::string(key), window, cx);
        }
        let local_table = forms.local_table.clone();
        let remote_table = forms.remote_table.clone();
        drop(forms);
        local_table.update(cx, |table, cx| {
            table.delegate_mut().refresh_localized_text();
            table.refresh(cx);
        });
        remote_table.update(cx, |table, cx| {
            table.delegate_mut().refresh_localized_text();
            table.refresh(cx);
        });
    }

    pub(in crate::ui::shell) fn new(
        args: SftpControllerArgs,
        session_query: SessionQueryPort,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let forms = Self::build_forms(
            args.local_hidden_columns,
            args.remote_hidden_columns,
            window,
            cx,
        );
        let local_path_input = forms.local_path_input.clone();
        let remote_path_input = forms.remote_path_input.clone();
        let local_table = forms.local_table.clone();
        let remote_table = forms.remote_table.clone();
        let prompt_input = forms.prompt_input.clone();
        let inline_rename_input = forms.inline_rename_input.clone();
        let controller = cx.weak_entity();

        local_table.update(cx, |table, _| {
            table.delegate_mut().set_controller(controller.clone());
        });
        remote_table.update(cx, |table, _| {
            table.delegate_mut().set_controller(controller);
        });

        let local_path_subscription =
            cx.subscribe(&local_path_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.commit_local_path(cx);
                }
            });
        let remote_path_subscription =
            cx.subscribe(&remote_path_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.commit_remote_path(cx);
                }
            });
        let local_table_subscription = cx.subscribe_in(
            &local_table,
            window,
            |this, table, event: &TableEvent, window, cx| {
                this.handle_local_table_event(table, event, window, cx);
            },
        );
        let remote_table_subscription = cx.subscribe_in(
            &remote_table,
            window,
            |this, table, event: &TableEvent, window, cx| {
                this.handle_remote_table_event(table, event, window, cx);
            },
        );
        let prompt_subscription = cx.subscribe(&prompt_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let Some(tab_id) = this.browser_tab_id(cx) else {
                    return;
                };
                this.commit_prompt(tab_id, cx);
            }
        });
        let inline_rename_subscription = cx.subscribe(
            &inline_rename_input,
            |this, _, event: &InputEvent, cx| match event {
                InputEvent::PressEnter { .. } => this.commit_inline_rename(cx),
                InputEvent::Blur => this.cancel_inline_rename(cx),
                _ => {}
            },
        );

        Self {
            service: args.service,
            session_query,
            forms: RefCell::new(forms),
            tabs: RefCell::new(SftpTabStore::default()),
            interactions: RefCell::new(HashMap::new()),
            progress_layout: RefCell::new(SftpProgressLayoutState::default()),
            download_destination_prompt_tab: Cell::new(None),
            _subscriptions: vec![
                local_path_subscription,
                remote_path_subscription,
                local_table_subscription,
                remote_table_subscription,
                prompt_subscription,
                inline_rename_subscription,
            ],
        }
    }

    pub(in crate::ui::shell) fn local_path_input(&self) -> Entity<InputState> {
        self.forms.borrow().local_path_input.clone()
    }

    pub(in crate::ui::shell) fn remote_path_input(&self) -> Entity<InputState> {
        self.forms.borrow().remote_path_input.clone()
    }

    pub(in crate::ui::shell) fn local_table(&self) -> Entity<TableState<SftpBrowserTableDelegate>> {
        self.forms.borrow().local_table.clone()
    }

    pub(in crate::ui::shell) fn remote_table(
        &self,
    ) -> Entity<TableState<SftpBrowserTableDelegate>> {
        self.forms.borrow().remote_table.clone()
    }

    fn browser_tab_id(&self, cx: &gpui::App) -> Option<TabId> {
        self.remote_table()
            .read(cx)
            .delegate()
            .tab_id()
            .or_else(|| {
                self.interactions
                    .borrow()
                    .iter()
                    .find_map(|(tab_id, state)| state.prompt.as_ref().map(|_| *tab_id))
            })
    }

    fn handle_local_table_event(
        &mut self,
        table_entity: &Entity<TableState<SftpBrowserTableDelegate>>,
        event: &TableEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = table_entity.read(cx).delegate().tab_id() else {
            return;
        };

        match event {
            TableEvent::SelectRow(row_ix) => {
                let modifiers = table_entity.update(cx, |table, _| {
                    table.delegate_mut().take_pending_select_modifiers()
                });
                self.select_local_row(tab_id, *row_ix, modifiers, cx);
            }
            TableEvent::RightClickedRow(Some(row_ix)) => {
                let Some(row) = table_entity.read(cx).delegate().row(*row_ix) else {
                    return;
                };
                let clicked_path = PathBuf::from(row.path.as_str());
                let keep_existing_selection = self.tab(tab_id).is_some_and(|tab| {
                    tab.selected_local_paths
                        .iter()
                        .any(|selected| selected == &clicked_path)
                });
                if !keep_existing_selection {
                    self.select_local_path(tab_id, clicked_path, cx);
                }
            }
            TableEvent::DoubleClickedRow(row_ix) => {
                let Some((path, is_directory)) = table_entity
                    .read(cx)
                    .delegate()
                    .row(*row_ix)
                    .map(|row| (PathBuf::from(row.path.as_str()), row.is_directory))
                else {
                    return;
                };
                self.select_local_path(tab_id, path.clone(), cx);
                if is_directory {
                    self.navigate_local_into_selected(tab_id, cx);
                } else {
                    self.queue_upload_path(tab_id, path, cx);
                }
            }
            TableEvent::ClearSelection => {
                table_entity.update(cx, |table, cx| {
                    table.delegate_mut().set_selected_paths(Vec::new(), None);
                    table.set_right_clicked_row(None, cx);
                });
                self.clear_local_selection(tab_id, cx);
            }
            _ => {}
        }
    }

    fn handle_remote_table_event(
        &mut self,
        table_entity: &Entity<TableState<SftpBrowserTableDelegate>>,
        event: &TableEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = table_entity.read(cx).delegate().tab_id() else {
            return;
        };

        match event {
            TableEvent::SelectRow(row_ix) => {
                let modifiers = table_entity.update(cx, |table, _| {
                    table.delegate_mut().take_pending_select_modifiers()
                });
                self.select_remote_row(tab_id, *row_ix, modifiers, cx);
            }
            TableEvent::RightClickedRow(Some(row_ix)) => {
                let Some(row) = table_entity.read(cx).delegate().row(*row_ix) else {
                    return;
                };
                let clicked_path = row.path.clone();
                let keep_existing_selection = self.tab(tab_id).is_some_and(|tab| {
                    tab.selected_remote_paths
                        .iter()
                        .any(|selected| selected == &clicked_path)
                });
                if !keep_existing_selection {
                    self.select_remote_path(tab_id, clicked_path, cx);
                }
            }
            TableEvent::DoubleClickedRow(row_ix) => {
                let Some((remote_path, is_directory)) = table_entity
                    .read(cx)
                    .delegate()
                    .row(*row_ix)
                    .map(|row| (row.path.clone(), row.is_directory))
                else {
                    return;
                };
                self.select_remote_path(tab_id, remote_path.clone(), cx);
                if is_directory {
                    self.navigate_remote_into_selected(tab_id, cx);
                } else {
                    let prompt_for_destination =
                        self.download_destination_prompt_tab.get() == Some(tab_id);
                    self.queue_download_path(
                        tab_id,
                        remote_path,
                        prompt_for_destination,
                        window,
                        cx,
                    );
                }
            }
            TableEvent::ClearSelection => {
                table_entity.update(cx, |table, cx| {
                    table.delegate_mut().set_selected_paths(Vec::new(), None);
                    table.set_right_clicked_row(None, cx);
                });
                self.clear_remote_selection(tab_id, cx);
            }
            _ => {}
        }
    }

    pub(in crate::ui::shell) fn prompt_input(&self) -> Entity<InputState> {
        self.forms.borrow().prompt_input.clone()
    }

    pub(in crate::ui::shell) fn inline_rename_input(&self) -> Entity<InputState> {
        self.forms.borrow().inline_rename_input.clone()
    }

    pub(in crate::ui::shell) fn request_expand_directory(
        &mut self,
        tab_id: TabId,
        side: SftpBrowserSide,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.expand_directory(tab_id, side, path, cx);
    }

    pub(in crate::ui::shell) fn local_path_editing(&self) -> bool {
        self.forms.borrow().local_path_editing
    }

    pub(in crate::ui::shell) fn remote_path_editing(&self) -> bool {
        self.forms.borrow().remote_path_editing
    }

    pub(in crate::ui::shell) fn remote_path_submit_pending(&self) -> bool {
        self.forms.borrow().remote_path_submit_pending
    }

    pub(in crate::ui::shell) fn set_local_path_editing(&self, editing: bool) {
        self.forms.borrow_mut().local_path_editing = editing;
    }

    pub(in crate::ui::shell) fn set_remote_path_editing(&self, editing: bool) {
        self.forms.borrow_mut().remote_path_editing = editing;
    }

    pub(in crate::ui::shell) fn set_remote_path_submit_pending(&self, pending: bool) {
        self.forms.borrow_mut().remote_path_submit_pending = pending;
    }

    pub(in crate::ui::shell) fn display_local_path(path: &std::path::Path) -> SharedString {
        SftpService::display_local_path(path).into()
    }

    pub(in crate::ui::shell) fn sync_path_inputs_for_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((local_path, remote_path)) = self.tab(tab_id).map(|tab| {
            (
                Self::display_local_path(&tab.local_path),
                SharedString::from(tab.remote_path.clone()),
            )
        }) else {
            return;
        };
        let local_input = self.local_path_input();
        let remote_input = self.remote_path_input();
        local_input.update(cx, |input, cx| {
            input.set_value(local_path, window, cx);
        });
        remote_input.update(cx, |input, cx| {
            input.set_value(remote_path, window, cx);
        });
    }

    pub(in crate::ui::shell) fn set_path_editing(
        &mut self,
        side: SftpBrowserSide,
        editing: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = match side {
            SftpBrowserSide::Local if self.local_path_editing() != editing => {
                self.set_local_path_editing(editing);
                true
            }
            SftpBrowserSide::Remote if self.remote_path_editing() != editing => {
                self.set_remote_path_editing(editing);
                true
            }
            _ => false,
        };
        if !changed {
            return;
        }

        cx.notify();
        if editing {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::ZERO).await;
                let _ = this.update(cx, |controller, cx| {
                    controller.focus_path_input_in_active_window(side, cx);
                });
            })
            .detach();
        }
    }

    pub(in crate::ui::shell) fn reset_path_editing(&self) {
        self.set_local_path_editing(false);
        self.set_remote_path_editing(false);
        self.set_remote_path_submit_pending(false);
    }

    fn commit_local_path(&mut self, cx: &mut Context<Self>) {
        let value = self.local_path_input().read(cx).value().trim().to_string();
        let Some(tab_id) = self.browser_tab_id(cx) else {
            return;
        };
        let Some(current_path) = self.tab(tab_id).map(|tab| tab.local_path.clone()) else {
            return;
        };

        let next_path = if value.is_empty() {
            current_path
        } else {
            let candidate = PathBuf::from(&value);
            if candidate.is_absolute() {
                candidate
            } else {
                current_path.join(candidate)
            }
        };

        if !next_path.exists() || !next_path.is_dir() {
            let path = next_path.display().to_string();
            self.notify_validation_failure(
                ValidationNotificationKind::InvalidInput,
                i18n::string_args("sftp.messages.local_path_not_directory", &[("path", &path)]),
                cx,
            );
            return;
        }

        let normalized = next_path.canonicalize().unwrap_or(next_path);
        self.set_local_path_editing(false);
        self.navigate_local_to_path(tab_id, normalized, cx);
    }

    fn commit_remote_path(&mut self, cx: &mut Context<Self>) {
        let value = self.remote_path_input().read(cx).value().trim().to_string();
        let Some(tab_id) = self.browser_tab_id(cx) else {
            return;
        };
        let Some(current_path) = self.tab(tab_id).map(|tab| tab.remote_path.clone()) else {
            return;
        };

        let next_path = if value.is_empty() {
            current_path
        } else if value.starts_with('/') {
            value
        } else {
            Self::join_remote_path(&current_path, &value)
        };

        self.request_remote_directory_with_source(tab_id, next_path, true, cx);
    }

    pub(in crate::ui::shell) fn navigate_local_to_path(
        &mut self,
        tab_id: TabId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let normalized = path.canonicalize().unwrap_or(path);
        if let Some(mut tab) = self.tab_mut(tab_id) {
            tab.local_path = normalized;
            tab.selected_local_path = None;
            tab.selected_local_paths.clear();
        }
        self.refresh_local_directory(tab_id, cx);
    }

    pub(in crate::ui::shell) fn navigate_local_into_selected(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(next_path) = self
            .tab(tab_id)
            .and_then(|tab| tab.selected_local_path.clone())
        else {
            return;
        };
        if next_path.is_dir() {
            self.navigate_local_to_path(tab_id, next_path, cx);
        }
    }

    pub(in crate::ui::shell) fn open_local_path(
        &mut self,
        tab_id: TabId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if path.is_dir() {
            self.navigate_local_to_path(tab_id, path, cx);
            return;
        }

        if let Err(error) = open::that(&path) {
            let path = path.display().to_string();
            let error = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "sftp.messages.open_local_failed",
                &[("path", &path), ("error", &error)],
            )));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn navigate_local_up(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(next_path) = self
            .tab(tab_id)
            .and_then(|tab| tab.local_path.parent().map(Path::to_path_buf))
        else {
            return;
        };
        self.navigate_local_to_path(tab_id, next_path, cx);
    }

    pub(in crate::ui::shell) fn request_remote_directory(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.request_remote_directory_with_source(tab_id, path, false, cx);
    }

    fn request_remote_directory_with_source(
        &mut self,
        tab_id: TabId,
        path: String,
        from_path_input: bool,
        cx: &mut Context<Self>,
    ) {
        self.set_remote_path_submit_pending(from_path_input);
        let (remote_loading, request_error) = {
            let Some(mut tab) = self.tab_mut(tab_id) else {
                return;
            };

            tab.pending_remote_refresh_paths.clear();
            tab.loading_remote = true;
            tab.last_error = None;
            let mut request_error = None;
            if let Some(commands) = tab.commands.as_ref() {
                match commands.list_directory(path.clone()) {
                    Ok(request_id) => {
                        tab.remote_directory_request_id = Some(request_id);
                        tab.requested_remote_path = Some(path);
                    }
                    Err(error) => {
                        let error = error.to_string();
                        tab.loading_remote = false;
                        tab.last_error = Some(error.clone());
                        request_error = Some(error);
                    }
                }
            }
            (tab.loading_remote, request_error)
        };

        if let Some(error) = request_error {
            let message = i18n::string_args("sftp.messages.refresh_failed", &[("error", &error)]);
            cx.emit(AppCommand::Feedback(message.clone()));
            if from_path_input {
                self.set_remote_path_submit_pending(false);
                self.notify_validation_failure(
                    ValidationNotificationKind::InvalidInput,
                    message,
                    cx,
                );
            }
        }

        let remote_table = self.remote_table();
        if remote_table.read(cx).delegate().tab_id() == Some(tab_id) {
            remote_table.update(cx, |table, cx| {
                table.delegate_mut().set_loading(remote_loading);
                table.refresh(cx);
            });
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn navigate_remote_into_selected(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(selected_path) = self
            .tab(tab_id)
            .and_then(|tab| tab.selected_remote_path.clone())
        else {
            return;
        };
        let Some(entry) = self.resolve_remote_entry(tab_id, &selected_path, cx) else {
            return;
        };
        if entry.kind == miaominal_sftp::SftpEntryKind::Directory {
            self.request_remote_directory(tab_id, entry.path, cx);
        }
    }

    pub(in crate::ui::shell) fn navigate_remote_up(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(current_path) = self.tab(tab_id).map(|tab| tab.remote_path.clone()) else {
            return;
        };
        self.request_remote_directory(tab_id, Self::remote_parent_path(&current_path), cx);
    }

    pub(in crate::ui::shell) fn begin_inline_rename(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selected_path) = self.tab(tab_id).and_then(|tab| {
            (tab.selected_remote_paths.len() == 1)
                .then(|| tab.selected_remote_path.clone())
                .flatten()
        }) else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "status.sftp.rename_requires_single_remote_entry",
            )));
            cx.notify();
            return;
        };
        let Some(entry) = self.resolve_remote_entry(tab_id, &selected_path, cx) else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "status.sftp.rename_requires_single_remote_entry",
            )));
            cx.notify();
            return;
        };

        let from = entry.path.clone();
        self.set_inline_rename(
            tab_id,
            Some(InlineRenameState::Remote {
                from: from.clone(),
                parent: Self::remote_parent_path(&entry.path),
            }),
        );
        let input = self.inline_rename_input();
        set_input_value(&input, entry.filename, window, cx);
        self.remote_table().update(cx, |table, cx| {
            table.delegate_mut().inline_rename_path = Some(from);
            table.refresh(cx);
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(in crate::ui::shell) fn begin_local_inline_rename(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selected_path) = self.tab(tab_id).and_then(|tab| {
            (tab.selected_local_paths.len() == 1)
                .then(|| tab.selected_local_path.clone())
                .flatten()
        }) else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "status.sftp.rename_requires_single_local_entry",
            )));
            cx.notify();
            return;
        };
        let Some(parent) = selected_path.parent().map(Path::to_path_buf) else {
            return;
        };
        let Some(filename) = selected_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            return;
        };

        let display_path = selected_path.display().to_string();
        self.set_inline_rename(
            tab_id,
            Some(InlineRenameState::Local {
                from: selected_path,
                parent,
            }),
        );
        let input = self.inline_rename_input();
        set_input_value(&input, filename, window, cx);
        self.local_table().update(cx, |table, cx| {
            table.delegate_mut().inline_rename_path = Some(display_path);
            table.refresh(cx);
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn commit_inline_rename(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.browser_tab_id(cx) else {
            return;
        };
        let Some(rename_state) = self.inline_rename(tab_id) else {
            return;
        };
        let value = self
            .inline_rename_input()
            .read(cx)
            .value()
            .trim()
            .to_string();
        if value.is_empty() {
            self.notify_validation_failure(
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("errors.sftp.validation.name_required"),
                cx,
            );
            return;
        }

        match rename_state {
            InlineRenameState::Local { from, parent } => {
                let to = parent.join(&value);
                if from == to {
                    self.clear_inline_rename(tab_id, cx);
                    cx.emit(AppCommand::Feedback(i18n::string(
                        "sftp.messages.name_unchanged",
                    )));
                    cx.notify();
                    return;
                }

                match std::fs::rename(&from, &to) {
                    Ok(()) => {
                        self.clear_inline_rename(tab_id, cx);
                        let from_display = from.display().to_string();
                        let to_display = to.display().to_string();
                        cx.emit(AppCommand::Feedback(i18n::string_args(
                            "sftp.messages.renaming",
                            &[("from", &from_display), ("to", &to_display)],
                        )));
                        self.select_local_path(tab_id, to, cx);
                        self.refresh_local_directory(tab_id, cx);
                    }
                    Err(error) => {
                        let error = error.to_string();
                        cx.emit(AppCommand::Feedback(i18n::string_args(
                            "sftp.messages.action_failed",
                            &[("error", &error)],
                        )));
                        cx.notify();
                    }
                }
            }
            InlineRenameState::Remote { from, parent } => {
                let Some(commands) = self.tab(tab_id).and_then(|tab| tab.commands.clone()) else {
                    return;
                };
                let to = Self::join_remote_path(&parent, &value);
                if from == to {
                    self.clear_inline_rename(tab_id, cx);
                    cx.emit(AppCommand::Feedback(i18n::string(
                        "sftp.messages.name_unchanged",
                    )));
                    cx.notify();
                    return;
                }

                match commands.rename(from.clone(), to.clone()) {
                    Ok(()) => {
                        self.clear_inline_rename(tab_id, cx);
                        cx.emit(AppCommand::Feedback(i18n::string_args(
                            "sftp.messages.renaming",
                            &[("from", &from), ("to", &to)],
                        )));
                        self.request_remote_directory(tab_id, parent, cx);
                    }
                    Err(error) => {
                        let error = error.to_string();
                        cx.emit(AppCommand::Feedback(i18n::string_args(
                            "sftp.messages.action_failed",
                            &[("error", &error)],
                        )));
                        cx.notify();
                    }
                }
            }
        }
    }

    pub(in crate::ui::shell) fn cancel_inline_rename(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.browser_tab_id(cx) else {
            return;
        };
        if self.inline_rename(tab_id).is_some() {
            self.clear_inline_rename(tab_id, cx);
            cx.notify();
        }
    }

    fn clear_inline_rename(&self, tab_id: TabId, cx: &mut Context<Self>) {
        self.set_inline_rename(tab_id, None);
        self.local_table().update(cx, |table, cx| {
            table.delegate_mut().inline_rename_path = None;
            table.refresh(cx);
        });
        self.remote_table().update(cx, |table, cx| {
            table.delegate_mut().inline_rename_path = None;
            table.refresh(cx);
        });
    }

    fn remote_parent_path(path: &str) -> String {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() || trimmed == "/" {
            return "/".into();
        }
        match trimmed.rsplit_once('/') {
            Some(("", _)) | None => "/".into(),
            Some((parent, _)) => parent.to_string(),
        }
    }

    pub(in crate::ui::shell) fn refresh_local_directory(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some((path, generation)) = self.tab_mut(tab_id).map(|mut tab| {
            tab.local_scan_generation = tab.local_scan_generation.wrapping_add(1);
            (tab.local_path.clone(), tab.local_scan_generation)
        }) else {
            return;
        };

        cx.spawn(async move |this, cx| {
            let scan_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { Self::read_local_entries(&scan_path) })
                .await;
            let _ = this.update(cx, move |this, cx| {
                let Some(mut tab) = this.tab_mut(tab_id) else {
                    return;
                };
                if tab.local_scan_generation != generation || tab.local_path != path {
                    return;
                }

                match result {
                    Ok(entries) => {
                        tab.local_entries = entries;
                        let available_paths = tab
                            .local_entries
                            .iter()
                            .map(|entry| entry.path.clone())
                            .collect::<HashSet<_>>();
                        tab.selected_local_paths
                            .retain(|selected| available_paths.contains(selected));
                        if let Some(selected) = tab.selected_local_path.as_ref()
                            && !tab
                                .local_entries
                                .iter()
                                .any(|entry| &entry.path == selected)
                        {
                            tab.selected_local_path = None;
                        }
                        if tab.selected_local_path.is_none() {
                            tab.selected_local_path = tab.selected_local_paths.first().cloned();
                        }
                        if let Some(selected) = tab.selected_local_path.clone()
                            && !tab
                                .selected_local_paths
                                .iter()
                                .any(|path| path == &selected)
                        {
                            tab.selected_local_paths.insert(0, selected);
                        }
                        if let Some(anchor) = tab.local_selection_anchor.as_ref()
                            && !tab.local_entries.iter().any(|entry| &entry.path == anchor)
                        {
                            tab.local_selection_anchor = None;
                        }
                        drop(tab);
                        this.sync_visible_path_input_in_active_window(
                            tab_id,
                            SftpBrowserSide::Local,
                            cx,
                        );
                        this.sync_table_for_side(tab_id, SftpBrowserSide::Local, cx);
                    }
                    Err(error) => {
                        tab.last_error = Some(error.to_string());
                        let path = tab.local_path.display().to_string();
                        let error = error.to_string();
                        drop(tab);
                        cx.emit(AppCommand::Feedback(i18n::string_args(
                            "sftp.messages.local_read_failed",
                            &[("path", &path), ("error", &error)],
                        )));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn local_attributes(metadata: &std::fs::Metadata) -> Option<String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            return Some(format!("{:o}", metadata.permissions().mode() & 0o777));
        }
        #[cfg(not(unix))]
        {
            Some(if metadata.permissions().readonly() {
                i18n::string("sftp.attributes.readonly")
            } else {
                i18n::string("sftp.attributes.read_write")
            })
        }
    }

    fn local_owner(metadata: &std::fs::Metadata) -> Option<String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            return Some(format!("{}:{}", metadata.uid(), metadata.gid()));
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            None
        }
    }

    fn read_local_entries(path: &Path) -> Result<Vec<LocalSftpEntry>> {
        let mut entries = Vec::new();
        let directory_path = path.display().to_string();
        for entry in std::fs::read_dir(path).with_context(|| {
            i18n::string_args(
                "errors.sftp.local_read.directory",
                &[("path", &directory_path)],
            )
        })? {
            let entry = entry.with_context(|| {
                i18n::string_args("errors.sftp.local_read.entry", &[("path", &directory_path)])
            })?;
            let entry_path = entry.path();
            let entry_path_text = entry_path.display().to_string();
            let metadata = entry.metadata().with_context(|| {
                i18n::string_args(
                    "errors.sftp.local_read.metadata",
                    &[("path", &entry_path_text)],
                )
            })?;
            entries.push(LocalSftpEntry {
                filename: entry.file_name().to_string_lossy().into_owned(),
                is_directory: metadata.is_dir(),
                size: metadata.is_file().then_some(metadata.len()),
                modified: metadata.modified().ok(),
                attributes: Self::local_attributes(&metadata),
                owner: Self::local_owner(&metadata),
                path: entry_path,
            });
        }

        entries.sort_by(|left, right| {
            right.is_directory.cmp(&left.is_directory).then_with(|| {
                left.filename
                    .to_lowercase()
                    .cmp(&right.filename.to_lowercase())
            })
        });
        Ok(entries)
    }

    pub(in crate::ui::shell) fn expand_directory(
        &mut self,
        tab_id: TabId,
        side: SftpBrowserSide,
        path: String,
        cx: &mut Context<Self>,
    ) {
        match side {
            SftpBrowserSide::Local => {
                let path_buf = PathBuf::from(&path);
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { Self::read_local_entries(&path_buf) })
                        .await;
                    let _ = this.update(cx, move |this, cx| {
                        if this.tab(tab_id).is_none() {
                            return;
                        }
                        let local_table = this.local_table();
                        match result {
                            Ok(entries) => {
                                let children = entries
                                    .iter()
                                    .map(SftpBrowserTableRow::from_local)
                                    .collect::<Vec<_>>();
                                local_table.update(cx, |table, cx| {
                                    if table.delegate().tab_id() == Some(tab_id) {
                                        table.delegate_mut().receive_children(path, children, cx);
                                    }
                                });
                            }
                            Err(error) => {
                                let error = error.to_string();
                                cx.emit(AppCommand::Feedback(i18n::string_args(
                                    "status.sftp.expand_local_failed",
                                    &[("path", &path), ("error", &error)],
                                )));
                                local_table.update(cx, |table, cx| {
                                    if table.delegate().tab_id() == Some(tab_id) {
                                        table.delegate_mut().cancel_expand(&path);
                                        cx.notify();
                                    }
                                });
                            }
                        }
                    });
                })
                .detach();
            }
            SftpBrowserSide::Remote => {
                let commands = self.tab(tab_id).and_then(|tab| tab.commands.clone());
                if let Some(commands) = commands.as_ref()
                    && let Err(error) = commands.list_subdirectory(&path)
                {
                    let error = error.to_string();
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "status.sftp.expand_remote_failed",
                        &[("path", &path), ("error", &error)],
                    )));
                    self.remote_table().update(cx, |table, cx| {
                        table.delegate_mut().cancel_expand(&path);
                        cx.notify();
                    });
                }
                cx.notify();
            }
        }
    }

    fn receive_subdirectory_listing(
        &mut self,
        tab_id: TabId,
        parent_path: String,
        entries: Vec<SftpEntry>,
        cx: &mut Context<Self>,
    ) {
        if self.remote_table().read(cx).delegate().tab_id() != Some(tab_id) {
            return;
        }
        let children = entries
            .iter()
            .map(SftpBrowserTableRow::from_remote)
            .collect::<Vec<_>>();
        self.remote_table().update(cx, |table, cx| {
            table
                .delegate_mut()
                .receive_children(parent_path, children, cx);
        });
    }

    fn schedule_directory_refresh(
        &mut self,
        tab_id: TabId,
        local_path: Option<PathBuf>,
        remote_path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };
        if let Some(path) = local_path {
            tab.pending_local_refresh_paths.insert(path);
        }
        if let Some(path) = remote_path {
            tab.pending_remote_refresh_paths.insert(path);
        }
        tab.directory_refresh_generation = tab.directory_refresh_generation.wrapping_add(1);
        let generation = tab.directory_refresh_generation;
        drop(tab);

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(SFTP_DIRECTORY_REFRESH_DEBOUNCE)
                .await;
            let _ = this.update(cx, move |this, cx| {
                let Some(mut tab) = this.tab_mut(tab_id) else {
                    return;
                };
                if tab.directory_refresh_generation != generation {
                    return;
                }

                let local_paths = std::mem::take(&mut tab.pending_local_refresh_paths);
                let remote_paths = std::mem::take(&mut tab.pending_remote_refresh_paths);
                let refresh_local = local_paths.contains(&tab.local_path);
                let remote_path = Self::debounced_remote_refresh_target(
                    &remote_paths,
                    &tab.remote_path,
                    tab.requested_remote_path.as_deref(),
                );
                drop(tab);

                if refresh_local {
                    this.refresh_local_directory(tab_id, cx);
                }
                if let Some(path) = remote_path {
                    this.request_remote_directory(tab_id, path, cx);
                }
            });
        })
        .detach();
    }

    fn transfer_remote_parent_path(path: &str) -> String {
        let path = path.trim_end_matches('/');
        match path.rsplit_once('/') {
            Some(("", _)) => "/".to_string(),
            Some((parent, _)) if !parent.is_empty() => parent.to_string(),
            _ => ".".to_string(),
        }
    }

    fn is_current_directory_response(
        current: Option<SftpDirectoryRequestId>,
        response: Option<SftpDirectoryRequestId>,
    ) -> bool {
        current == response
    }

    fn debounced_remote_refresh_target(
        pending_paths: &HashSet<String>,
        loaded_path: &str,
        requested_path: Option<&str>,
    ) -> Option<String> {
        let target = requested_path.unwrap_or(loaded_path);
        pending_paths.contains(target).then(|| target.to_string())
    }

    pub(in crate::ui::shell) fn sync_tables_for_tab(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        self.sync_table_for_side(tab_id, SftpBrowserSide::Local, cx);
        self.sync_table_for_side(tab_id, SftpBrowserSide::Remote, cx);
    }

    pub(in crate::ui::shell) fn set_download_destination_prompt_tab(
        &self,
        tab_id: TabId,
        enabled: bool,
    ) {
        self.download_destination_prompt_tab
            .set(enabled.then_some(tab_id));
    }

    pub(in crate::ui::shell) fn sync_table_for_side(
        &mut self,
        tab_id: TabId,
        side: SftpBrowserSide,
        cx: &mut Context<Self>,
    ) {
        match side {
            SftpBrowserSide::Local => {
                let Some((rows, selected_paths, selected_path)) = self.tab(tab_id).map(|tab| {
                    (
                        tab.local_entries
                            .iter()
                            .map(SftpBrowserTableRow::from_local)
                            .collect::<Vec<_>>(),
                        tab.selected_local_paths
                            .iter()
                            .map(|selected| selected.display().to_string())
                            .collect::<Vec<_>>(),
                        tab.selected_local_path
                            .as_ref()
                            .map(|selected| selected.display().to_string()),
                    )
                }) else {
                    return;
                };
                self.local_table().update(cx, |table, cx| {
                    table.delegate_mut().set_rows(rows, false, tab_id);
                    table
                        .delegate_mut()
                        .set_selected_paths(selected_paths, selected_path);
                    table.set_right_clicked_row(None, cx);
                    table.refresh(cx);
                });
            }
            SftpBrowserSide::Remote => {
                let Some((rows, selected_paths, selected_path, loading)) =
                    self.tab(tab_id).map(|tab| {
                        (
                            tab.remote_entries
                                .iter()
                                .map(SftpBrowserTableRow::from_remote)
                                .collect::<Vec<_>>(),
                            tab.selected_remote_paths.clone(),
                            tab.selected_remote_path.clone(),
                            tab.loading_remote,
                        )
                    })
                else {
                    return;
                };
                self.remote_table().update(cx, |table, cx| {
                    table.delegate_mut().set_rows(rows, loading, tab_id);
                    table
                        .delegate_mut()
                        .set_selected_paths(selected_paths, selected_path);
                    table.set_right_clicked_row(None, cx);
                    table.refresh(cx);
                });
            }
        }
    }

    pub(in crate::ui::shell) fn sync_selection_for_tab(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some((local_paths, local_path, remote_paths, remote_path)) =
            self.tab(tab_id).map(|tab| {
                (
                    tab.selected_local_paths
                        .iter()
                        .map(|selected| selected.display().to_string())
                        .collect::<Vec<_>>(),
                    tab.selected_local_path
                        .as_ref()
                        .map(|selected| selected.display().to_string()),
                    tab.selected_remote_paths.clone(),
                    tab.selected_remote_path.clone(),
                )
            })
        else {
            return;
        };
        self.local_table().update(cx, |table, cx| {
            table
                .delegate_mut()
                .set_selected_paths(local_paths, local_path);
            table.set_right_clicked_row(None, cx);
            cx.notify();
        });
        self.remote_table().update(cx, |table, cx| {
            table
                .delegate_mut()
                .set_selected_paths(remote_paths, remote_path);
            table.set_right_clicked_row(None, cx);
            cx.notify();
        });
    }

    pub(in crate::ui::shell) fn sync_selection_for_side(
        &mut self,
        tab_id: TabId,
        side: SftpBrowserSide,
        cx: &mut Context<Self>,
    ) {
        match side {
            SftpBrowserSide::Local => {
                let Some((selected_paths, selected_path)) = self.tab(tab_id).map(|tab| {
                    (
                        tab.selected_local_paths
                            .iter()
                            .map(|selected| selected.display().to_string())
                            .collect::<Vec<_>>(),
                        tab.selected_local_path
                            .as_ref()
                            .map(|selected| selected.display().to_string()),
                    )
                }) else {
                    return;
                };
                self.local_table().update(cx, |table, cx| {
                    table
                        .delegate_mut()
                        .set_selected_paths(selected_paths, selected_path);
                    table.set_right_clicked_row(None, cx);
                    cx.notify();
                });
            }
            SftpBrowserSide::Remote => {
                let Some((selected_paths, selected_path)) = self.tab(tab_id).map(|tab| {
                    (
                        tab.selected_remote_paths.clone(),
                        tab.selected_remote_path.clone(),
                    )
                }) else {
                    return;
                };
                self.remote_table().update(cx, |table, cx| {
                    table
                        .delegate_mut()
                        .set_selected_paths(selected_paths, selected_path);
                    table.set_right_clicked_row(None, cx);
                    cx.notify();
                });
            }
        }
    }

    pub(in crate::ui::shell) fn clear_local_selection(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        self.set_local_selection(tab_id, Vec::new(), None, cx);
    }

    pub(in crate::ui::shell) fn clear_remote_selection(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        self.set_remote_selection(tab_id, Vec::new(), None, cx);
    }

    pub(in crate::ui::shell) fn handle_blank_click(
        &mut self,
        tab_id: TabId,
        side: SftpBrowserSide,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        header_height: Pixels,
        cx: &mut Context<Self>,
    ) {
        if position.y <= bounds.origin.y + header_height {
            return;
        }

        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };

        let suppress_click = match side {
            SftpBrowserSide::Local => &mut tab.suppress_local_clear_click,
            SftpBrowserSide::Remote => &mut tab.suppress_remote_clear_click,
        };
        if *suppress_click {
            *suppress_click = false;
            return;
        }

        let has_selection = match side {
            SftpBrowserSide::Local => !tab.selected_local_paths.is_empty(),
            SftpBrowserSide::Remote => !tab.selected_remote_paths.is_empty(),
        };
        drop(tab);

        if !has_selection {
            return;
        }

        match side {
            SftpBrowserSide::Local => self.clear_local_selection(tab_id, cx),
            SftpBrowserSide::Remote => self.clear_remote_selection(tab_id, cx),
        }
        self.sync_selection_for_tab(tab_id, cx);
    }

    pub(in crate::ui::shell) fn select_local_row(
        &mut self,
        tab_id: TabId,
        row_ix: usize,
        modifiers: SftpBrowserSelectionModifiers,
        cx: &mut Context<Self>,
    ) {
        let Some(clicked_path) = self
            .local_table()
            .read(cx)
            .delegate()
            .row(row_ix)
            .map(|row| PathBuf::from(row.path.as_str()))
        else {
            return;
        };

        if modifiers.shift {
            let (anchor, existing_paths) = self
                .tab(tab_id)
                .map(|tab| {
                    (
                        tab.local_selection_anchor
                            .clone()
                            .or_else(|| tab.selected_local_path.clone())
                            .unwrap_or_else(|| clicked_path.clone()),
                        tab.selected_local_paths.clone(),
                    )
                })
                .unwrap_or_else(|| (clicked_path.clone(), Vec::new()));
            let range_paths = self.local_paths_in_click_range(&anchor, row_ix, cx);
            let mut next_paths = if modifiers.toggle {
                existing_paths
            } else {
                Vec::new()
            };

            for path in range_paths {
                if !next_paths.iter().any(|current| current == &path) {
                    next_paths.push(path);
                }
            }

            self.set_local_selection(tab_id, next_paths, Some(clicked_path), cx);
            self.set_local_selection_anchor(tab_id, Some(anchor));
        } else if modifiers.toggle {
            let (mut next_paths, current_primary) = self
                .tab(tab_id)
                .map(|tab| {
                    (
                        tab.selected_local_paths.clone(),
                        tab.selected_local_path.clone(),
                    )
                })
                .unwrap_or_default();

            let was_selected = next_paths.iter().any(|path| path == &clicked_path);
            if was_selected {
                next_paths.retain(|path| path != &clicked_path);
            } else {
                next_paths.push(clicked_path.clone());
            }

            let next_primary = if next_paths.is_empty() {
                None
            } else if was_selected {
                current_primary
                    .filter(|primary| next_paths.iter().any(|path| path == primary))
                    .or_else(|| next_paths.first().cloned())
            } else {
                Some(clicked_path.clone())
            };
            let next_anchor = (!next_paths.is_empty()).then_some(clicked_path);

            self.set_local_selection(tab_id, next_paths, next_primary, cx);
            self.set_local_selection_anchor(tab_id, next_anchor);
        } else {
            self.select_local_path(tab_id, clicked_path, cx);
        }

        self.sync_selection_for_side(tab_id, SftpBrowserSide::Local, cx);
    }

    pub(in crate::ui::shell) fn select_remote_row(
        &mut self,
        tab_id: TabId,
        row_ix: usize,
        modifiers: SftpBrowserSelectionModifiers,
        cx: &mut Context<Self>,
    ) {
        let Some(clicked_path) = self
            .remote_table()
            .read(cx)
            .delegate()
            .row(row_ix)
            .map(|row| row.path.clone())
        else {
            return;
        };

        if modifiers.shift {
            let (anchor, existing_paths) = self
                .tab(tab_id)
                .map(|tab| {
                    (
                        tab.remote_selection_anchor
                            .clone()
                            .or_else(|| tab.selected_remote_path.clone())
                            .unwrap_or_else(|| clicked_path.clone()),
                        tab.selected_remote_paths.clone(),
                    )
                })
                .unwrap_or_else(|| (clicked_path.clone(), Vec::new()));
            let range_paths = self.remote_paths_in_click_range(&anchor, row_ix, cx);
            let mut next_paths = if modifiers.toggle {
                existing_paths
            } else {
                Vec::new()
            };

            for path in range_paths {
                if !next_paths.iter().any(|current| current == &path) {
                    next_paths.push(path);
                }
            }

            self.set_remote_selection(tab_id, next_paths, Some(clicked_path), cx);
            self.set_remote_selection_anchor(tab_id, Some(anchor));
        } else if modifiers.toggle {
            let (mut next_paths, current_primary) = self
                .tab(tab_id)
                .map(|tab| {
                    (
                        tab.selected_remote_paths.clone(),
                        tab.selected_remote_path.clone(),
                    )
                })
                .unwrap_or_default();

            let was_selected = next_paths.iter().any(|path| path == &clicked_path);
            if was_selected {
                next_paths.retain(|path| path != &clicked_path);
            } else {
                next_paths.push(clicked_path.clone());
            }

            let next_primary = if next_paths.is_empty() {
                None
            } else if was_selected {
                current_primary
                    .filter(|primary| next_paths.iter().any(|path| path == primary))
                    .or_else(|| next_paths.first().cloned())
            } else {
                Some(clicked_path.clone())
            };
            let next_anchor = (!next_paths.is_empty()).then_some(clicked_path);

            self.set_remote_selection(tab_id, next_paths, next_primary, cx);
            self.set_remote_selection_anchor(tab_id, next_anchor);
        } else {
            self.select_remote_path(tab_id, clicked_path, cx);
        }

        self.sync_selection_for_side(tab_id, SftpBrowserSide::Remote, cx);
    }

    fn local_paths_in_click_range(
        &self,
        anchor: &std::path::Path,
        row_ix: usize,
        cx: &gpui::App,
    ) -> Vec<PathBuf> {
        let table = self.local_table().read(cx);
        let delegate = table.delegate();
        let anchor_key = anchor.display().to_string();
        let anchor_ix = delegate.row_index_by_path(&anchor_key).unwrap_or(row_ix);

        delegate
            .paths_in_row_range(anchor_ix, row_ix)
            .into_iter()
            .map(PathBuf::from)
            .collect()
    }

    fn remote_paths_in_click_range(
        &self,
        anchor: &str,
        row_ix: usize,
        cx: &gpui::App,
    ) -> Vec<String> {
        let table = self.remote_table().read(cx);
        let delegate = table.delegate();
        let anchor_ix = delegate.row_index_by_path(anchor).unwrap_or(row_ix);

        delegate.paths_in_row_range(anchor_ix, row_ix)
    }

    fn set_local_selection_anchor(&self, tab_id: TabId, anchor: Option<PathBuf>) {
        if let Some(mut tab) = self.tab_mut(tab_id) {
            tab.local_selection_anchor = anchor;
        }
    }

    fn set_remote_selection_anchor(&self, tab_id: TabId, anchor: Option<String>) {
        if let Some(mut tab) = self.tab_mut(tab_id) {
            tab.remote_selection_anchor = anchor;
        }
    }

    pub(in crate::ui::shell) fn set_local_selection(
        &mut self,
        tab_id: TabId,
        paths: Vec<PathBuf>,
        primary: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };

        let mut unique_paths = Vec::new();
        for path in paths {
            if !unique_paths.iter().any(|current| current == &path) {
                unique_paths.push(path);
            }
        }

        let primary = primary.or_else(|| unique_paths.first().cloned());
        if let Some(primary_path) = primary.clone()
            && !unique_paths.iter().any(|path| path == &primary_path)
        {
            unique_paths.insert(0, primary_path.clone());
        }

        if unique_paths.is_empty() {
            tab.local_selection_anchor = None;
        }
        if tab.selected_local_path == primary && tab.selected_local_paths == unique_paths {
            return;
        }

        tab.selected_local_path = primary;
        tab.selected_local_paths = unique_paths;
        drop(tab);
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_remote_selection(
        &mut self,
        tab_id: TabId,
        paths: Vec<String>,
        primary: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };

        let mut unique_paths = Vec::new();
        for path in paths {
            if !unique_paths.iter().any(|current| current == &path) {
                unique_paths.push(path);
            }
        }

        let primary = primary.or_else(|| unique_paths.first().cloned());
        if let Some(primary_path) = primary.clone()
            && !unique_paths.iter().any(|path| path == &primary_path)
        {
            unique_paths.insert(0, primary_path.clone());
        }

        if unique_paths.is_empty() {
            tab.remote_selection_anchor = None;
        }
        if tab.selected_remote_path == primary && tab.selected_remote_paths == unique_paths {
            return;
        }

        tab.selected_remote_path = primary;
        tab.selected_remote_paths = unique_paths;
        drop(tab);
        cx.notify();
    }

    pub(in crate::ui::shell) fn select_local_path(
        &mut self,
        tab_id: TabId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.set_local_selection(tab_id, vec![path.clone()], Some(path.clone()), cx);
        self.set_local_selection_anchor(tab_id, Some(path));
    }

    pub(in crate::ui::shell) fn select_remote_path(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.set_remote_selection(tab_id, vec![path.clone()], Some(path.clone()), cx);
        self.set_remote_selection_anchor(tab_id, Some(path));
    }

    pub(in crate::ui::shell) fn begin_drag_selection(
        &mut self,
        tab_id: TabId,
        side: SftpBrowserSide,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        header_height: Pixels,
        cx: &mut Context<Self>,
    ) {
        if position.y <= bounds.origin.y + header_height {
            return;
        }

        let relative_position =
            Point::new(position.x - bounds.origin.x, position.y - bounds.origin.y);
        let scroll_offset = self.drag_selection_scroll_offset(side, cx);
        let anchor_content_y =
            relative_position.y.as_f32() - header_height.as_f32() + scroll_offset;

        let generation = {
            let Some(mut tab) = self.tab_mut(tab_id) else {
                return;
            };

            tab.drag_selection_generation = tab.drag_selection_generation.wrapping_add(1);
            let generation = tab.drag_selection_generation;
            tab.drag_selection_context = Some(SftpDragSelectionContext {
                side,
                tab_id,
                last_position: position,
                panel_bounds: bounds,
                row_height: header_height,
                anchor_content_y,
                generation,
            });

            match side {
                SftpBrowserSide::Local => {
                    tab.local_drag_candidate = Some(relative_position);
                    tab.local_drag_selection = None;
                    tab.suppress_local_clear_click = false;
                }
                SftpBrowserSide::Remote => {
                    tab.remote_drag_candidate = Some(relative_position);
                    tab.remote_drag_selection = None;
                    tab.suppress_remote_clear_click = false;
                }
            }

            generation
        };

        self.start_drag_selection_auto_scroll(tab_id, generation, cx);
    }

    pub(in crate::ui::shell) fn update_active_drag_selection(
        &mut self,
        tab_id: TabId,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(context) = self.drag_selection_context_for_tab(tab_id) else {
            return false;
        };

        self.update_drag_selection(
            context.tab_id,
            context.side,
            position,
            context.panel_bounds,
            context.row_height,
            cx,
        )
    }

    pub(in crate::ui::shell) fn finish_active_drag_selection(
        &mut self,
        tab_id: TabId,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(context) = self.drag_selection_context_for_tab(tab_id) else {
            return false;
        };

        self.finish_drag_selection(
            context.tab_id,
            context.side,
            position,
            context.panel_bounds,
            context.row_height,
            cx,
        )
    }

    pub(in crate::ui::shell) fn finish_any_active_drag_selection(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        let contexts = {
            let tabs = self.tabs();
            tabs.iter()
                .filter_map(|(tab_id, tab)| {
                    let context = tab.drag_selection_context?;
                    (context.tab_id == *tab_id).then_some(context)
                })
                .collect::<Vec<_>>()
        };

        if contexts.is_empty() {
            return false;
        }

        for context in contexts {
            self.finish_drag_selection(
                context.tab_id,
                context.side,
                context.last_position,
                context.panel_bounds,
                context.row_height,
                cx,
            );
        }

        cx.notify();
        true
    }

    fn drag_selection_context_for_tab(&self, tab_id: TabId) -> Option<SftpDragSelectionContext> {
        self.tab(tab_id)
            .and_then(|tab| tab.drag_selection_context)
            .filter(|context| context.tab_id == tab_id)
    }

    fn clear_drag_selection_context(&mut self, tab_id: TabId) -> bool {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return false;
        };

        let had_context = tab.drag_selection_context.take().is_some();
        if had_context {
            tab.drag_selection_generation = tab.drag_selection_generation.wrapping_add(1);
        }
        had_context
    }

    fn drag_selection_scroll_offset(&self, side: SftpBrowserSide, cx: &gpui::App) -> f32 {
        let offset = match side {
            SftpBrowserSide::Local => {
                self.local_table()
                    .read(cx)
                    .vertical_scroll_handle
                    .offset()
                    .y
            }
            SftpBrowserSide::Remote => {
                self.remote_table()
                    .read(cx)
                    .vertical_scroll_handle
                    .offset()
                    .y
            }
        };

        -offset.as_f32()
    }

    fn drag_selection_relative_position(
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        row_height: Pixels,
    ) -> Point<Pixels> {
        let body_top = bounds.origin.y + row_height;
        let body_bottom = (bounds.origin.y + bounds.size.height).max(body_top);
        let clamped_y = position.y.max(body_top).min(body_bottom);

        Point::new(position.x - bounds.origin.x, clamped_y - bounds.origin.y)
    }

    fn start_drag_selection_auto_scroll(
        &mut self,
        tab_id: TabId,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(SFTP_DRAG_AUTO_SCROLL_INTERVAL)
                    .await;

                let keep_scrolling = this
                    .update(cx, |this, cx| {
                        this.tick_drag_selection_auto_scroll(tab_id, generation, cx)
                    })
                    .unwrap_or(false);

                if !keep_scrolling {
                    break;
                }
            }
        })
        .detach();
    }

    fn tick_drag_selection_auto_scroll(
        &mut self,
        tab_id: TabId,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(context) = self.drag_selection_context_for_tab(tab_id) else {
            return false;
        };
        if context.generation != generation {
            return false;
        }

        let selection_active = self.tab(tab_id).is_some_and(|tab| match context.side {
            SftpBrowserSide::Local => tab.local_drag_selection.is_some(),
            SftpBrowserSide::Remote => tab.remote_drag_selection.is_some(),
        });
        if !selection_active {
            return true;
        }

        let Some(step) = Self::drag_selection_auto_scroll_step(context) else {
            return true;
        };

        if self.scroll_drag_selection_table(context, step, cx) {
            self.update_drag_selection(
                context.tab_id,
                context.side,
                context.last_position,
                context.panel_bounds,
                context.row_height,
                cx,
            );
        }

        true
    }

    fn drag_selection_auto_scroll_step(context: SftpDragSelectionContext) -> Option<f32> {
        let body_top = (context.panel_bounds.origin.y + context.row_height).as_f32();
        let body_bottom =
            (context.panel_bounds.origin.y + context.panel_bounds.size.height).as_f32();
        if body_bottom <= body_top {
            return None;
        }

        let edge_zone = SFTP_DRAG_AUTO_SCROLL_EDGE_ZONE.min((body_bottom - body_top) / 2.0);
        if edge_zone < 1.0 {
            return None;
        }

        let pointer_y = context.last_position.y.as_f32();
        let hot_top = body_top + edge_zone;
        let hot_bottom = body_bottom - edge_zone;

        let signed_distance = if pointer_y < hot_top {
            -(hot_top - pointer_y)
        } else if pointer_y > hot_bottom {
            pointer_y - hot_bottom
        } else {
            return None;
        };

        let ratio = (signed_distance.abs() / edge_zone).clamp(0.0, SFTP_DRAG_AUTO_SCROLL_MAX_RATIO);
        let eased = ratio.powf(1.2);
        Some(
            (eased * SFTP_DRAG_AUTO_SCROLL_MAX_STEP).max(SFTP_DRAG_AUTO_SCROLL_MIN_STEP)
                * signed_distance.signum(),
        )
    }

    fn scroll_drag_selection_table(
        &mut self,
        context: SftpDragSelectionContext,
        step: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        match context.side {
            SftpBrowserSide::Local => self
                .local_table()
                .update(cx, |table, cx| Self::scroll_table_by_step(table, step, cx)),
            SftpBrowserSide::Remote => self
                .remote_table()
                .update(cx, |table, cx| Self::scroll_table_by_step(table, step, cx)),
        }
    }

    fn scroll_table_by_step(
        table: &mut TableState<SftpBrowserTableDelegate>,
        step: f32,
        cx: &mut Context<TableState<SftpBrowserTableDelegate>>,
    ) -> bool {
        let current_offset = table.vertical_scroll_handle.offset();
        let max_offset = table
            .vertical_scroll_handle
            .0
            .borrow()
            .base_handle
            .max_offset();
        let next_y = (current_offset.y.as_f32() - step).clamp(-max_offset.y.as_f32(), 0.0);

        if (next_y - current_offset.y.as_f32()).abs() < 0.5 {
            return false;
        }

        table
            .vertical_scroll_handle
            .set_offset(Point::new(current_offset.x, px(next_y)));
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn update_drag_selection(
        &mut self,
        tab_id: TabId,
        side: SftpBrowserSide,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        row_height: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let relative_position =
            Self::drag_selection_relative_position(position, bounds, row_height);
        let scroll_offset = self.drag_selection_scroll_offset(side, cx);

        let drag = {
            let Some(mut tab) = self.tab_mut(tab_id) else {
                return false;
            };

            let anchor_view_y = if let Some(context) = tab.drag_selection_context.as_mut()
                && context.tab_id == tab_id
                && context.side == side
            {
                context.last_position = position;
                context.panel_bounds = bounds;
                context.row_height = row_height;
                Some(px(
                    context.anchor_content_y - scroll_offset + row_height.as_f32()
                ))
            } else {
                None
            };

            let drag = match side {
                SftpBrowserSide::Local => tab.local_drag_selection.as_mut(),
                SftpBrowserSide::Remote => tab.remote_drag_selection.as_mut(),
            };

            if let Some(drag) = drag {
                if let Some(anchor_view_y) = anchor_view_y {
                    drag.start.y = anchor_view_y;
                }
                drag.update(relative_position);
                Some((*drag, false))
            } else {
                let candidate = match side {
                    SftpBrowserSide::Local => tab.local_drag_candidate,
                    SftpBrowserSide::Remote => tab.remote_drag_candidate,
                };

                candidate.and_then(|candidate_start| {
                    let mut state = SftpDragSelectionState::new(candidate_start);
                    if let Some(anchor_view_y) = anchor_view_y {
                        state.start.y = anchor_view_y;
                    }
                    state.update(relative_position);
                    state
                        .exceeds_threshold(px(SFTP_DRAG_SELECTION_THRESHOLD))
                        .then(|| {
                            match side {
                                SftpBrowserSide::Local => {
                                    tab.local_drag_candidate = None;
                                    tab.local_drag_selection = Some(state);
                                }
                                SftpBrowserSide::Remote => {
                                    tab.remote_drag_candidate = None;
                                    tab.remote_drag_selection = Some(state);
                                }
                            }
                            (state, true)
                        })
                })
            }
        };

        let Some((drag, force_selection_update)) = drag else {
            return false;
        };

        self.apply_drag_selection(tab_id, side, drag, row_height, force_selection_update, cx);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn finish_drag_selection(
        &mut self,
        tab_id: TabId,
        side: SftpBrowserSide,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        row_height: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let relative_position =
            Self::drag_selection_relative_position(position, bounds, row_height);
        let scroll_offset = self.drag_selection_scroll_offset(side, cx);

        let drag = {
            let Some(mut tab) = self.tab_mut(tab_id) else {
                return false;
            };

            match side {
                SftpBrowserSide::Local => tab.local_drag_candidate = None,
                SftpBrowserSide::Remote => tab.remote_drag_candidate = None,
            }

            let mut drag = match side {
                SftpBrowserSide::Local => tab.local_drag_selection.take(),
                SftpBrowserSide::Remote => tab.remote_drag_selection.take(),
            };

            if let Some(context) = tab.drag_selection_context
                && context.tab_id == tab_id
                && context.side == side
                && let Some(drag) = drag.as_mut()
            {
                drag.start.y = px(context.anchor_content_y - scroll_offset + row_height.as_f32());
            }

            drag
        };

        self.clear_drag_selection_context(tab_id);

        let Some(mut drag) = drag else {
            return false;
        };

        drag.update(relative_position);
        if !drag.exceeds_threshold(px(SFTP_DRAG_SELECTION_THRESHOLD)) {
            cx.notify();
            return false;
        }

        self.apply_drag_selection(tab_id, side, drag, row_height, true, cx);
        if let Some(mut tab) = self.tab_mut(tab_id) {
            match side {
                SftpBrowserSide::Local => tab.suppress_local_clear_click = true,
                SftpBrowserSide::Remote => tab.suppress_remote_clear_click = true,
            }
        }
        cx.notify();
        true
    }

    fn apply_drag_selection(
        &mut self,
        tab_id: TabId,
        side: SftpBrowserSide,
        drag: SftpDragSelectionState,
        row_height: Pixels,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let (row_range, selected_paths) = self.drag_selection_paths(side, drag, row_height, cx);

        let row_range_changed = {
            let Some(mut tab) = self.tab_mut(tab_id) else {
                return;
            };

            let drag = match side {
                SftpBrowserSide::Local => tab.local_drag_selection.as_mut(),
                SftpBrowserSide::Remote => tab.remote_drag_selection.as_mut(),
            };

            if force {
                if let Some(drag) = drag {
                    drag.set_last_row_range(row_range);
                }
                true
            } else {
                drag.is_some_and(|drag| drag.set_last_row_range(row_range))
            }
        };

        if !row_range_changed {
            return;
        }

        match side {
            SftpBrowserSide::Local => {
                let selected_paths: Vec<PathBuf> =
                    selected_paths.into_iter().map(PathBuf::from).collect();
                let primary = selected_paths.first().cloned();
                self.set_local_selection(tab_id, selected_paths, primary, cx);
            }
            SftpBrowserSide::Remote => {
                let primary = selected_paths.first().cloned();
                self.set_remote_selection(tab_id, selected_paths, primary, cx);
            }
        }

        self.sync_selection_for_side(tab_id, side, cx);
    }

    fn drag_selection_paths(
        &self,
        side: SftpBrowserSide,
        drag: SftpDragSelectionState,
        row_height: Pixels,
        cx: &gpui::App,
    ) -> (Option<(usize, usize)>, Vec<String>) {
        match side {
            SftpBrowserSide::Local => {
                let table = self.local_table().read(cx);
                let row_range = Self::drag_selection_row_range(table, drag, row_height);
                let selected_paths = row_range
                    .map(|(start, end)| table.delegate().paths_in_row_range(start, end))
                    .unwrap_or_default();
                (row_range, selected_paths)
            }
            SftpBrowserSide::Remote => {
                let table = self.remote_table().read(cx);
                let row_range = Self::drag_selection_row_range(table, drag, row_height);
                let selected_paths = row_range
                    .map(|(start, end)| table.delegate().paths_in_row_range(start, end))
                    .unwrap_or_default();
                (row_range, selected_paths)
            }
        }
    }

    fn drag_selection_row_range(
        table: &TableState<SftpBrowserTableDelegate>,
        drag: SftpDragSelectionState,
        row_height: Pixels,
    ) -> Option<(usize, usize)> {
        let row_count = table.delegate().row_count();
        if row_count == 0 || row_height <= px(0.0) {
            return None;
        }

        let row_height_px = row_height.as_f32();
        let bounds = drag.bounds();
        let body_top = row_height_px;
        let scroll_offset = -table.vertical_scroll_handle.offset().y.as_f32();
        let content_top = bounds.origin.y.as_f32() - body_top + scroll_offset;
        let content_bottom =
            bounds.origin.y.as_f32() + bounds.size.height.as_f32() - body_top + scroll_offset;
        let total_height = row_count as f32 * row_height_px;

        if content_bottom < 0.0 || content_top >= total_height {
            return None;
        }

        let content_top = content_top.clamp(0.0, total_height);
        let content_bottom = content_bottom.clamp(0.0, total_height);
        let start_row = (content_top / row_height_px)
            .floor()
            .clamp(0.0, row_count.saturating_sub(1) as f32) as usize;
        let end_y = if content_bottom <= content_top {
            content_top
        } else {
            content_bottom - 0.1
        };
        let end_row = (end_y / row_height_px)
            .floor()
            .clamp(0.0, row_count.saturating_sub(1) as f32) as usize;

        Some((start_row, end_row))
    }

    pub(in crate::ui::shell) fn session_progress_visible(&self) -> bool {
        self.progress_layout.borrow().session_visible
    }

    pub(in crate::ui::shell) fn set_session_progress_visible(&self, visible: bool) -> bool {
        let started_at = Instant::now();
        let mut layout = self.progress_layout.borrow_mut();
        let SftpProgressLayoutState {
            session_visible,
            session_transition,
            ..
        } = &mut *layout;
        update_progress_center_state(session_visible, session_transition, visible, started_at)
    }

    pub(in crate::ui::shell) fn apply_progress_visibility(&self, visible: bool) -> bool {
        let started_at = Instant::now();
        let mut changed = {
            let mut layout = self.progress_layout.borrow_mut();
            let SftpProgressLayoutState {
                session_visible,
                session_transition,
                ..
            } = &mut *layout;
            update_progress_center_state(session_visible, session_transition, visible, started_at)
        };

        for tab in self.tabs_mut().values_mut() {
            if !update_progress_center_state(
                &mut tab.layout.progress_center_visible,
                &mut tab.layout.progress_center_transition,
                visible,
                started_at,
            ) {
                continue;
            }

            if !visible
                && matches!(
                    tab.layout.drag.as_ref(),
                    Some(drag) if drag.divider == SftpSplitDivider::ProgressCenter
                )
            {
                tab.layout.drag = None;
            }
            changed = true;
        }

        changed
    }

    pub(in crate::ui::shell) fn session_progress_render_visibility(
        &self,
        window: &mut Window,
    ) -> Option<f32> {
        let mut layout = self.progress_layout.borrow_mut();
        let SftpProgressLayoutState {
            session_visible,
            session_transition,
            ..
        } = &mut *layout;
        progress_center_render_visibility(*session_visible, session_transition, window)
    }

    pub(in crate::ui::shell) fn tab_progress_render_visibility(
        &self,
        tab_id: TabId,
        window: &mut Window,
    ) -> Option<f32> {
        let mut tab = self.tab_mut(tab_id)?;
        progress_center_render_visibility(
            tab.layout.progress_center_visible,
            &mut tab.layout.progress_center_transition,
            window,
        )
    }

    pub(in crate::ui::shell) fn progress_scroll_handle(&self) -> ScrollHandle {
        self.progress_layout.borrow().scroll_handle.clone()
    }

    pub(in crate::ui::shell) fn session_progress_flex(&self) -> f32 {
        self.progress_layout.borrow().session_flex
    }

    pub(in crate::ui::shell) fn set_session_progress_flex(&self, flex: f32) {
        self.progress_layout.borrow_mut().session_flex = flex;
    }

    pub(in crate::ui::shell) fn session_progress_drag(
        &self,
    ) -> Option<SessionSftpProgressCenterDragState> {
        self.progress_layout.borrow().session_drag.clone()
    }

    pub(in crate::ui::shell) fn set_session_progress_drag(
        &self,
        drag: Option<SessionSftpProgressCenterDragState>,
    ) {
        self.progress_layout.borrow_mut().session_drag = drag;
    }

    pub(in crate::ui::shell) fn take_session_progress_drag(
        &self,
    ) -> Option<SessionSftpProgressCenterDragState> {
        self.progress_layout.borrow_mut().session_drag.take()
    }

    pub(in crate::ui::shell) fn tab(&self, tab_id: TabId) -> Option<Ref<'_, SftpTabState>> {
        let tabs = self.tabs.borrow();
        Ref::filter_map(tabs, |store| store.tabs.get(&tab_id)).ok()
    }

    pub(in crate::ui::shell) fn tab_mut(&self, tab_id: TabId) -> Option<RefMut<'_, SftpTabState>> {
        let tabs = self.tabs.borrow_mut();
        RefMut::filter_map(tabs, |store| store.tabs.get_mut(&tab_id)).ok()
    }

    pub(in crate::ui::shell) fn tabs(&self) -> Ref<'_, HashMap<TabId, SftpTabState>> {
        Ref::map(self.tabs.borrow(), |store| &store.tabs)
    }

    pub(in crate::ui::shell) fn tabs_mut(&self) -> RefMut<'_, HashMap<TabId, SftpTabState>> {
        RefMut::map(self.tabs.borrow_mut(), |store| &mut store.tabs)
    }

    pub(in crate::ui::shell) fn session_profiles(&self) -> Vec<SessionProfile> {
        self.session_query.profiles()
    }

    fn insert_tab(&self, tab_id: TabId, tab: SftpTabState) {
        self.tabs.borrow_mut().insert(tab_id, tab);
    }

    pub(in crate::ui::shell) fn start_tab(
        &mut self,
        tab_id: TabId,
        profile: SessionProfile,
        owner: Option<TabId>,
        cx: &mut Context<Self>,
    ) -> Result<SessionProfile, String> {
        let owner_profile =
            owner.and_then(|owner| self.session_query.resolved_profile_for_session(owner));
        let profile = Self::resolve_start_profile(profile, owner, owner_profile)?;
        let profiles = self.session_query.profiles();
        let connection = self.service.start_session(profile.clone(), profiles);
        let mut tab = SftpTabState::new(&profile);
        tab.commands = Some(connection.commands);
        self.insert_tab(tab_id, tab);
        self.refresh_local_directory(tab_id, cx);
        self.start_worker_tasks(tab_id, connection.events, connection.progress, cx);
        Ok(profile)
    }

    fn resolve_start_profile(
        saved_profile: SessionProfile,
        owner: Option<TabId>,
        owner_profile: Option<SessionProfile>,
    ) -> Result<SessionProfile, String> {
        let Some(owner) = owner else {
            return Ok(saved_profile);
        };

        owner_profile
            .filter(|profile| profile.id == saved_profile.id)
            .ok_or_else(|| format!("missing active SFTP owner {owner}"))
    }

    fn start_worker_tasks(
        &mut self,
        tab_id: TabId,
        mut events: SftpEventReceiver,
        mut progress: SftpProgressReceiver,
        cx: &mut Context<Self>,
    ) {
        let event_task = cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this
                    .update(cx, |this, cx| {
                        this.handle_event(tab_id, event, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let progress_task = cx.spawn(async move |this, cx| {
            while let Some(first) = progress.recv().await {
                cx.background_executor()
                    .timer(SFTP_PROGRESS_REFRESH_INTERVAL)
                    .await;

                let mut latest = HashMap::new();
                latest.insert(first.transfer_id, first);
                while let Some(update) = progress.try_recv() {
                    latest.insert(update.transfer_id, update);
                }

                if this
                    .update(cx, |this, cx| {
                        for update in latest.into_values() {
                            this.handle_progress(tab_id, update);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let interaction = self.interactions.get_mut().entry(tab_id).or_default();
        interaction.event_task = Some(event_task);
        interaction.progress_task = Some(progress_task);
    }

    fn handle_event(&mut self, tab_id: TabId, event: SftpEvent, cx: &mut Context<Self>) {
        if self.tab(tab_id).is_none() {
            return;
        }
        let is_visible_browser_tab =
            self.remote_table().read(cx).delegate().tab_id() == Some(tab_id);
        let remote_path_submit_pending =
            is_visible_browser_tab && self.remote_path_submit_pending();
        let edit_remote_path = match &event {
            SftpEvent::TransferDone { transfer_id } => {
                self.take_edit_pending_download(tab_id, *transfer_id)
            }
            _ => None,
        };

        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };
        let mut tab_status = None;
        let mut should_sync_paths = false;
        let mut refresh_local_directory = None;
        let mut refresh_remote_directory = None;
        let mut subdirectory_listing: Option<(String, Vec<SftpEntry>)> = None;
        let mut edit_complete: Option<(PathBuf, String)> = None;
        let mut validation_notification = None;
        let mut download_done_filename = None;
        let mut transfer_failed_notification = None;
        let mut open_global_progress_center = false;
        let mut remote_table_loading_finished = false;
        let mut clear_remote_table_loading = false;
        let mut failed_remote_expand_path = None;

        match event {
            SftpEvent::Status(message) => {
                tab_status = Some(message.clone());
                tab.last_status = message;
                tab.last_error = None;
            }
            SftpEvent::DirectoryListing {
                request_id,
                path,
                entries,
            } => {
                if !Self::is_current_directory_response(tab.remote_directory_request_id, request_id)
                {
                    return;
                }
                tab.remote_directory_request_id = None;
                tab.requested_remote_path = None;
                tab_status = Some(i18n::string_args(
                    "sftp.ui.remote_path_label",
                    &[("path", &path)],
                ));
                tab.remote_path = path;
                tab.remote_entries = entries;
                tab.selected_remote_path = None;
                tab.selected_remote_paths.clear();
                tab.remote_selection_anchor = None;
                tab.loading_remote = false;
                tab.last_error = None;
                let item_count = tab.remote_entries.len().to_string();
                tab.last_status = i18n::string_args(
                    "sftp.messages.loaded_remote_items",
                    &[("count", &item_count)],
                );
                tab.remote_drag_selection = None;
                tab.suppress_remote_clear_click = false;
                should_sync_paths = true;
            }
            SftpEvent::TransferQueued {
                transfer_id,
                direction,
                source,
                destination,
            } => {
                tab.transfers.insert(
                    0,
                    SftpTransferRow {
                        transfer_id,
                        direction,
                        source,
                        destination,
                        bytes_complete: 0,
                        bytes_total: None,
                        status: SftpTransferStatus::Queued,
                        bytes_per_second: None,
                        last_progress_at: None,
                        last_bytes_complete: 0,
                        is_directory: false,
                        expanded: false,
                        children: VecDeque::new(),
                        child_count: 0,
                    },
                );
                open_global_progress_center = true;
                let transfer_id = transfer_id.0.to_string();
                tab.last_status =
                    i18n::string_args("sftp.messages.transfer_queued", &[("id", &transfer_id)]);
            }
            SftpEvent::TransferChildStarted { transfer_id, child } => {
                if let Some(transfer) = tab
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.push_child(child);
                }
            }
            SftpEvent::TransferChildFinished { transfer_id, child } => {
                if let Some(transfer) = tab
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.apply_child_update(child);
                }
            }
            SftpEvent::TransferProgressFinal(progress) => {
                if let Some(transfer) = tab
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == progress.transfer_id)
                {
                    transfer.apply_progress(progress);
                }
            }
            SftpEvent::DirectoryListingFailed {
                request_id,
                path,
                message,
            } => {
                if !Self::is_current_directory_response(
                    tab.remote_directory_request_id,
                    Some(request_id),
                ) {
                    return;
                }
                tab.remote_directory_request_id = None;
                tab.requested_remote_path = None;
                tab.loading_remote = false;
                tab_status = Some(i18n::string("session.status.error"));
                tab.last_error = Some(format!("list_directory: {message}"));
                tab.last_status = i18n::string_args(
                    "sftp.messages.context_failed",
                    &[("context", "list_directory")],
                );
                remote_table_loading_finished = true;
                let notification_message = i18n::string_args(
                    "sftp.messages.operation_failed",
                    &[("context", "list_directory"), ("message", &message)],
                );
                cx.emit(AppCommand::Feedback(notification_message.clone()));
                if remote_path_submit_pending {
                    validation_notification = Some(notification_message);
                }
                log::debug!("failed to list requested SFTP directory {path}: {message}");
            }
            SftpEvent::TransferPaused { transfer_id } => {
                if let Some(transfer) = tab
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.status = SftpTransferStatus::Paused;
                    transfer.bytes_per_second = None;
                    transfer.last_progress_at = None;
                    transfer.pause_active_child();
                }
                let transfer_id = transfer_id.0.to_string();
                tab.last_status =
                    i18n::string_args("sftp.messages.transfer_paused", &[("id", &transfer_id)]);
            }
            SftpEvent::TransferResumed { transfer_id } => {
                if let Some(transfer) = tab
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.status = SftpTransferStatus::Running;
                    transfer.resume_active_child();
                }
                let transfer_id = transfer_id.0.to_string();
                tab.last_status =
                    i18n::string_args("sftp.messages.transfer_resumed", &[("id", &transfer_id)]);
            }
            SftpEvent::TransferDone { transfer_id } => {
                if let Some(transfer) = tab
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.status = SftpTransferStatus::Done;
                    transfer.bytes_per_second = None;
                    transfer.last_progress_at = None;
                    if let Some(total) = transfer.bytes_total {
                        transfer.bytes_complete = total;
                    }
                    match transfer.direction {
                        TransferDirection::Upload => {
                            refresh_remote_directory =
                                Some(Self::transfer_remote_parent_path(&transfer.destination));
                        }
                        TransferDirection::Download => {
                            if edit_remote_path.is_none() {
                                refresh_local_directory = Some(
                                    transfer
                                        .source
                                        .parent()
                                        .unwrap_or_else(|| Path::new("."))
                                        .to_path_buf(),
                                );
                                download_done_filename = Some(
                                    transfer
                                        .source
                                        .file_name()
                                        .map(|name| name.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| transfer.destination.clone()),
                                );
                            }
                        }
                    }
                }
                let transfer_id = transfer_id.0.to_string();
                tab.last_status =
                    i18n::string_args("sftp.messages.transfer_finished", &[("id", &transfer_id)]);

                if let Some(remote_path) = edit_remote_path {
                    let temp_path = std::env::temp_dir()
                        .join("miaominal_edit")
                        .join(tab_id.to_string())
                        .join(
                            Path::new(&remote_path)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "file".into()),
                        );
                    edit_complete = Some((temp_path, remote_path));
                }
            }
            SftpEvent::TransferCancelled { transfer_id } => {
                if let Some(transfer) = tab
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.status = SftpTransferStatus::Cancelled;
                    transfer.bytes_per_second = None;
                    transfer.last_progress_at = None;
                    transfer.cancel_unfinished_children();
                }
                let transfer_id = transfer_id.0.to_string();
                tab.last_status =
                    i18n::string_args("sftp.messages.transfer_cancelled", &[("id", &transfer_id)]);
            }
            SftpEvent::TransferFailed {
                transfer_id,
                message,
            } => {
                if let Some(transfer) = tab
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.status = SftpTransferStatus::Failed(message.clone());
                    transfer.bytes_per_second = None;
                    transfer.last_progress_at = None;
                    transfer.fail_unfinished_children(&message);
                }
                tab_status = Some(i18n::string("session.status.error"));
                tab.last_error = Some(message.clone());
                let transfer_id = transfer_id.0.to_string();
                tab.last_status =
                    i18n::string_args("sftp.messages.transfer_failed", &[("id", &transfer_id)]);
                transfer_failed_notification = Some(message.clone());
                cx.emit(AppCommand::Feedback(message));
            }
            SftpEvent::Error {
                context,
                path,
                message,
            } => {
                if context == "list_directory" && tab.remote_directory_request_id.is_some() {
                    return;
                }
                tab_status = Some(i18n::string("session.status.error"));
                if context == "list_directory" {
                    tab.loading_remote = false;
                    remote_table_loading_finished = true;
                } else if context == "list_subdirectory" {
                    failed_remote_expand_path = path;
                }
                tab.last_error = Some(format!("{context}: {message}"));
                tab.last_status =
                    i18n::string_args("sftp.messages.context_failed", &[("context", &context)]);
                let notification_message = i18n::string_args(
                    "sftp.messages.operation_failed",
                    &[("context", &context), ("message", &message)],
                );
                cx.emit(AppCommand::Feedback(notification_message.clone()));
                if remote_path_submit_pending {
                    validation_notification = Some(notification_message);
                }
            }
            SftpEvent::Closed => {
                tab_status = Some(i18n::string("session.status.closed"));
                tab.commands = None;
                tab.loading_remote = false;
                tab.last_status = i18n::string("sftp.messages.session_closed");
                clear_remote_table_loading = true;
            }
            SftpEvent::SubdirectoryListing {
                parent_path,
                entries,
            } => {
                if tab
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.starts_with("list_subdirectory:"))
                {
                    tab_status = Some(i18n::string_args(
                        "sftp.ui.remote_path_label",
                        &[("path", &tab.remote_path)],
                    ));
                    tab.last_error = None;
                    let item_count = entries.len().to_string();
                    tab.last_status = i18n::string_args(
                        "sftp.messages.loaded_remote_items",
                        &[("count", &item_count)],
                    );
                }
                subdirectory_listing = Some((parent_path, entries));
            }
        }
        drop(tab);

        if let Some(status) = tab_status {
            cx.emit(AppCommand::TabStatusChanged { tab_id, status });
        }

        if is_visible_browser_tab
            && (remote_table_loading_finished
                || clear_remote_table_loading
                || failed_remote_expand_path.is_some())
        {
            self.remote_table().update(cx, |table, cx| {
                if table.delegate().tab_id() != Some(tab_id) {
                    return;
                }
                if clear_remote_table_loading {
                    table.delegate_mut().cancel_all_loading();
                } else {
                    if remote_table_loading_finished {
                        table.delegate_mut().set_loading(false);
                    }
                    if let Some(path) = failed_remote_expand_path.as_deref() {
                        table.delegate_mut().cancel_expand(path);
                    }
                }
                table.refresh(cx);
            });
        }

        if should_sync_paths {
            self.sync_visible_path_input_in_active_window(tab_id, SftpBrowserSide::Remote, cx);
            self.sync_table_for_side(tab_id, SftpBrowserSide::Remote, cx);
            if is_visible_browser_tab {
                self.set_remote_path_editing(false);
                self.set_remote_path_submit_pending(false);
            }
        }

        if open_global_progress_center {
            self.apply_progress_visibility(true);
        }
        if refresh_local_directory.is_some() || refresh_remote_directory.is_some() {
            self.schedule_directory_refresh(
                tab_id,
                refresh_local_directory,
                refresh_remote_directory,
                cx,
            );
        }
        if let Some((parent_path, entries)) = subdirectory_listing {
            self.receive_subdirectory_listing(tab_id, parent_path, entries, cx);
        }
        if let Some((temp_path, remote_path)) = edit_complete {
            self.complete_edit_download(tab_id, temp_path, remote_path, cx);
        }
        if let Some(message) = validation_notification {
            self.set_remote_path_submit_pending(false);
            self.notify_validation_failure(ValidationNotificationKind::InvalidInput, message, cx);
            return;
        }
        if let Some(filename) = download_done_filename {
            self.notify_download_completed(filename, cx);
        }
        if let Some(message) = transfer_failed_notification {
            self.notify_transfer_failed(message, cx);
        }
        cx.notify();
    }

    fn handle_progress(&self, tab_id: TabId, progress: SftpTransferProgress) {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };
        let Some(transfer) = tab
            .transfers
            .iter_mut()
            .find(|transfer| transfer.transfer_id == progress.transfer_id)
        else {
            return;
        };
        transfer.apply_progress(progress);
    }

    pub(in crate::ui::shell) fn resolve_remote_entry(
        &self,
        tab_id: TabId,
        path: &str,
        cx: &gpui::App,
    ) -> Option<SftpEntry> {
        if let Some(entry) = self.tab(tab_id).and_then(|tab| {
            tab.remote_entries
                .iter()
                .find(|entry| entry.path == path)
                .cloned()
        }) {
            return Some(entry);
        }

        let row = {
            let table = self.remote_table().read(cx);
            let delegate = table.delegate();
            let row_ix = delegate.row_index_by_path(path)?;
            delegate.row(row_ix)?.clone()
        };

        let path = row.path.clone();
        Some(SftpEntry {
            filename: if row.name.as_ref().is_empty() {
                SftpService::remote_file_name(&path)
            } else {
                row.name.as_ref().to_string()
            },
            path,
            kind: if row.is_directory {
                miaominal_sftp::SftpEntryKind::Directory
            } else {
                row.kind
            },
            size: row.size,
            modified: row.modified,
            attributes: row.attributes.map(|value| value.to_string()),
            owner: row.owner.map(|value| value.to_string()),
        })
    }

    pub(in crate::ui::shell) fn queue_upload_selected(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(selected_paths) = self.tab(tab_id).map(|tab| tab.selected_local_paths.clone())
        else {
            return;
        };

        if selected_paths.is_empty() {
            return;
        }

        self.queue_upload_paths(tab_id, selected_paths, cx);
    }

    pub(in crate::ui::shell) fn queue_upload_path(
        &mut self,
        tab_id: TabId,
        local_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.queue_upload_paths(tab_id, vec![local_path], cx);
    }

    pub(in crate::ui::shell) fn queue_upload_paths(
        &mut self,
        tab_id: TabId,
        local_paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let Some((commands, remote_base, remote_entries)) = self.tab(tab_id).and_then(|tab| {
            Some((
                tab.commands.clone()?,
                tab.remote_path.clone(),
                tab.remote_entries.clone(),
            ))
        }) else {
            return;
        };

        let known_directory_paths = if local_paths.len() > 1 {
            self.local_table()
                .read(cx)
                .delegate()
                .directory_paths_in_selection(&local_paths)
        } else {
            Vec::new()
        };
        let uploads = SftpService::plan_uploads_with_known_directories(
            local_paths,
            &known_directory_paths,
            &remote_base,
        );
        let conflict_count = SftpService::count_remote_conflicts(&uploads, &remote_entries);

        if conflict_count > 0 {
            self.set_prompt(
                tab_id,
                Some(SftpPromptState {
                    kind: SftpPromptKind::ConfirmOverwrite {
                        conflict_count,
                        pending_uploads: uploads
                            .iter()
                            .map(|upload| (upload.local_path.clone(), upload.remote_path.clone()))
                            .collect(),
                        pending_downloads: Vec::new(),
                    },
                }),
            );
            cx.notify();
        } else if let Err(error) = SftpService::queue_uploads(&commands, &uploads) {
            let error = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "sftp.messages.upload_queue_failed",
                &[("error", &error)],
            )));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn queue_download_selected(
        &mut self,
        tab_id: TabId,
        prompt_for_destination: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(remote_paths) = self
            .tab(tab_id)
            .map(|tab| tab.selected_remote_paths.clone())
        else {
            return;
        };

        if remote_paths.is_empty() {
            return;
        }

        self.queue_download_paths(tab_id, remote_paths, prompt_for_destination, window, cx);
    }

    pub(in crate::ui::shell) fn queue_download_path(
        &mut self,
        tab_id: TabId,
        remote_path: String,
        prompt_for_destination: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.queue_download_paths(
            tab_id,
            vec![remote_path],
            prompt_for_destination,
            window,
            cx,
        );
    }

    fn queue_download_paths(
        &mut self,
        tab_id: TabId,
        remote_paths: Vec<String>,
        prompt_for_destination: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(local_base) = self
            .tab(tab_id)
            .and_then(|tab| tab.commands.as_ref().map(|_| tab.local_path.clone()))
        else {
            return;
        };

        let selected_entries: Vec<_> = remote_paths
            .into_iter()
            .filter_map(|remote| self.resolve_remote_entry(tab_id, &remote, cx))
            .collect();
        if selected_entries.is_empty() {
            return;
        }

        if prompt_for_destination {
            let destination =
                choose_sftp_download_destination(selected_entries, &local_base, window);
            cx.spawn(async move |this, cx| {
                let Some(prepared) = destination.await else {
                    return;
                };
                this.update(cx, |this, cx| {
                    this.queue_prepared_downloads(tab_id, prepared, cx);
                })
                .ok();
            })
            .detach();
            return;
        }

        self.queue_prepared_downloads(
            tab_id,
            PreparedSftpDownloads {
                downloads: SftpService::plan_downloads(selected_entries, &local_base),
                overwrite_confirmed: false,
            },
            cx,
        );
    }

    fn queue_prepared_downloads(
        &mut self,
        tab_id: TabId,
        prepared: PreparedSftpDownloads,
        cx: &mut Context<Self>,
    ) {
        let Some(commands) = self.tab(tab_id).and_then(|tab| tab.commands.clone()) else {
            return;
        };

        let pairs = prepared.downloads;
        let conflict_count = if prepared.overwrite_confirmed {
            0
        } else {
            pairs
                .iter()
                .filter(|download| download.local_path.exists())
                .count()
        };

        if conflict_count > 0 {
            self.set_prompt(
                tab_id,
                Some(SftpPromptState {
                    kind: SftpPromptKind::ConfirmOverwrite {
                        conflict_count,
                        pending_uploads: Vec::new(),
                        pending_downloads: pairs
                            .iter()
                            .map(|download| {
                                (download.remote_path.clone(), download.local_path.clone())
                            })
                            .collect(),
                    },
                }),
            );
            cx.notify();
        } else if let Err(error) = SftpService::queue_downloads(&commands, &pairs) {
            let error = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "sftp.messages.download_queue_failed",
                &[("error", &error)],
            )));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn pause_transfer(
        &mut self,
        tab_id: TabId,
        transfer_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        let Some(commands) = self.tab(tab_id).and_then(|tab| tab.commands.clone()) else {
            return;
        };
        if let Err(error) = SftpService::pause_transfer(&commands, transfer_id) {
            let error = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "sftp.messages.pause_transfer_failed",
                &[("error", &error)],
            )));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn resume_transfer(
        &mut self,
        tab_id: TabId,
        transfer_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        let Some(commands) = self.tab(tab_id).and_then(|tab| tab.commands.clone()) else {
            return;
        };
        if let Err(error) = SftpService::resume_transfer(&commands, transfer_id) {
            let error = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "sftp.messages.resume_transfer_failed",
                &[("error", &error)],
            )));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn cancel_transfer(
        &mut self,
        tab_id: TabId,
        transfer_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        let Some(commands) = self.tab(tab_id).and_then(|tab| tab.commands.clone()) else {
            return;
        };
        if let Err(error) = SftpService::cancel_transfer(&commands, transfer_id) {
            let error = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "sftp.messages.cancel_transfer_failed",
                &[("error", &error)],
            )));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn remove_transfer_record(
        &mut self,
        tab_id: TabId,
        transfer_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };
        let before = tab.transfers.len();
        tab.transfers
            .retain(|transfer| transfer.transfer_id != transfer_id);
        if tab.transfers.len() == before {
            return;
        }

        let transfer_id = transfer_id.0.to_string();
        tab.last_status = i18n::string_args(
            "sftp.messages.removed_transfer_record",
            &[("id", &transfer_id)],
        );
        drop(tab);
        cx.notify();
    }

    pub(in crate::ui::shell) fn toggle_transfer_expanded(
        &mut self,
        tab_id: TabId,
        transfer_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        let Some(mut tab) = self.tab_mut(tab_id) else {
            return;
        };
        let Some(transfer) = tab
            .transfers
            .iter_mut()
            .find(|transfer| transfer.transfer_id == transfer_id)
        else {
            return;
        };
        if transfer.children.is_empty() {
            return;
        }

        transfer.expanded = !transfer.expanded;
        drop(tab);
        cx.notify();
    }

    pub(in crate::ui::shell) fn begin_create_directory(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent) = self.tab(tab_id).map(|tab| tab.remote_path.clone()) else {
            return;
        };

        self.set_prompt(
            tab_id,
            Some(SftpPromptState {
                kind: SftpPromptKind::CreateRemoteDirectory { parent },
            }),
        );
        let input = self.prompt_input();
        set_input_placeholder(
            &input,
            i18n::string("sftp.prompts.directory_name_placeholder"),
            window,
            cx,
        );
        set_input_value(&input, "", window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn begin_create_local_directory(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent) = self.tab(tab_id).map(|tab| tab.local_path.clone()) else {
            return;
        };

        self.set_prompt(
            tab_id,
            Some(SftpPromptState {
                kind: SftpPromptKind::CreateLocalDirectory { parent },
            }),
        );
        let input = self.prompt_input();
        set_input_placeholder(
            &input,
            i18n::string("sftp.prompts.directory_name_placeholder"),
            window,
            cx,
        );
        set_input_value(&input, "", window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn delete_local_selected(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(entries) = self
            .tab(tab_id)
            .map(|tab| tab.selected_local_paths.clone())
            .filter(|entries| !entries.is_empty())
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "sftp.messages.select_local_entry_first",
            )));
            cx.notify();
            return;
        };

        self.set_prompt(
            tab_id,
            Some(SftpPromptState {
                kind: SftpPromptKind::ConfirmDeleteLocal { entries },
            }),
        );
        cx.notify();
    }

    fn execute_local_delete(
        &mut self,
        tab_id: TabId,
        entries: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { delete_local_entries(entries) })
                .await;

            let _ = this.update(cx, move |this, cx| {
                if result.deleted_count > 0 {
                    let message = if result.deleted_count == 1 {
                        i18n::string("sftp.messages.removed_one_local_entry")
                    } else {
                        let count = result.deleted_count.to_string();
                        i18n::string_args(
                            "sftp.messages.removed_local_entries",
                            &[("count", &count)],
                        )
                    };
                    cx.emit(AppCommand::Feedback(message));
                    this.refresh_local_directory(tab_id, cx);
                    cx.notify();
                    return;
                }

                if let Some((path, error)) = result.first_error {
                    let path = path.display().to_string();
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "sftp.messages.delete_failed_for",
                        &[("path", &path), ("error", &error)],
                    )));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(in crate::ui::shell) fn delete_remote_selected(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some((refresh_path, selected_paths)) = self.tab(tab_id).and_then(|tab| {
            tab.commands.as_ref()?;
            Some((tab.remote_path.clone(), tab.selected_remote_paths.clone()))
        }) else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "sftp.messages.select_remote_entry_first",
            )));
            cx.notify();
            return;
        };

        let selected_entries = normalize_remote_delete_entries(
            selected_paths
                .into_iter()
                .filter_map(|path| {
                    self.resolve_remote_entry(tab_id, &path, cx).map(|entry| {
                        (
                            entry.path,
                            entry.kind == miaominal_sftp::SftpEntryKind::Directory,
                        )
                    })
                })
                .collect::<Vec<_>>(),
        );

        if selected_entries.is_empty() {
            cx.emit(AppCommand::Feedback(i18n::string(
                "sftp.messages.select_remote_entry_first",
            )));
            cx.notify();
            return;
        }

        self.set_prompt(
            tab_id,
            Some(SftpPromptState {
                kind: SftpPromptKind::ConfirmDelete {
                    entries: selected_entries,
                    refresh_path,
                },
            }),
        );
        cx.notify();
    }

    fn execute_delete(
        &mut self,
        tab_id: TabId,
        commands: &SftpCommandSender,
        entries: Vec<(String, bool)>,
        refresh_path: String,
        cx: &mut Context<Self>,
    ) {
        let mut deleted_count = 0_usize;
        let mut first_error = None;

        for (path, is_directory) in entries {
            let result = if is_directory {
                commands.remove_directory(path.clone())
            } else {
                commands.remove_file(path.clone())
            };

            match result {
                Ok(()) => deleted_count += 1,
                Err(error) if first_error.is_none() => {
                    let error = error.to_string();
                    first_error = Some(i18n::string_args(
                        "sftp.messages.delete_failed_for",
                        &[("path", &path), ("error", &error)],
                    ));
                }
                Err(_) => {}
            }
        }

        if deleted_count > 0 {
            let message = if deleted_count == 1 {
                i18n::string("sftp.messages.removing_one_remote_entry")
            } else {
                let deleted_count = deleted_count.to_string();
                i18n::string_args(
                    "sftp.messages.removing_remote_entries",
                    &[("count", &deleted_count)],
                )
            };
            cx.emit(AppCommand::Feedback(message));
            self.request_remote_directory(tab_id, refresh_path, cx);
            cx.notify();
            return;
        }

        if let Some(error) = first_error {
            cx.emit(AppCommand::Feedback(error));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn skip_overwrite_prompt(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(commands) = self.tab(tab_id).and_then(|tab| tab.commands.clone()) else {
            return;
        };
        let Some(prompt) = self.prompt(tab_id) else {
            return;
        };
        let exit_snapshot = DialogOverlaySnapshot::SftpPrompt {
            tab_id,
            prompt: prompt.clone(),
        };
        let SftpPromptKind::ConfirmOverwrite {
            pending_uploads,
            pending_downloads,
            ..
        } = prompt.kind
        else {
            return;
        };
        let remote_entries = self
            .tab(tab_id)
            .map(|tab| tab.remote_entries.clone())
            .unwrap_or_default();

        self.take_prompt(tab_id);
        cx.emit(AppCommand::OverlayDismissed(exit_snapshot));

        for (local, remote) in pending_uploads {
            if remote_entries.iter().any(|entry| entry.path == remote) {
                continue;
            }
            if let Err(error) = commands.queue_upload(local, remote) {
                let error = error.to_string();
                cx.emit(AppCommand::Feedback(i18n::string_args(
                    "sftp.messages.upload_queue_failed",
                    &[("error", &error)],
                )));
                cx.notify();
                return;
            }
        }

        for (remote, local) in pending_downloads {
            if local.exists() {
                continue;
            }
            if let Err(error) = commands.queue_download(remote, local) {
                let error = error.to_string();
                cx.emit(AppCommand::Feedback(i18n::string_args(
                    "sftp.messages.download_queue_failed",
                    &[("error", &error)],
                )));
                cx.notify();
                return;
            }
        }

        cx.notify();
    }

    pub(in crate::ui::shell) fn commit_prompt(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt(tab_id) else {
            return;
        };
        let exit_snapshot = DialogOverlaySnapshot::SftpPrompt {
            tab_id,
            prompt: prompt.clone(),
        };

        if let SftpPromptKind::ConfirmDeleteLocal { entries } = prompt.kind {
            self.take_prompt(tab_id);
            cx.emit(AppCommand::OverlayDismissed(exit_snapshot));
            self.execute_local_delete(tab_id, entries, cx);
            return;
        }

        if let SftpPromptKind::CreateLocalDirectory { parent } = prompt.kind {
            let value = self.prompt_input().read(cx).value().trim().to_string();
            if value.is_empty() {
                self.notify_validation_failure(
                    ValidationNotificationKind::RequiredInputMissing,
                    i18n::string("errors.sftp.validation.name_required"),
                    cx,
                );
                return;
            }
            if !is_valid_local_directory_name(&value) {
                self.notify_validation_failure(
                    ValidationNotificationKind::InvalidInput,
                    i18n::string("errors.sftp.validation.directory_name_invalid"),
                    cx,
                );
                return;
            }

            match create_local_directory(&parent, &value) {
                Ok(path) => {
                    self.take_prompt(tab_id);
                    cx.emit(AppCommand::OverlayDismissed(exit_snapshot));
                    let display_path = path.display().to_string();
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "sftp.messages.created_local_directory",
                        &[("path", &display_path)],
                    )));
                    self.select_local_path(tab_id, path, cx);
                    self.refresh_local_directory(tab_id, cx);
                    cx.notify();
                }
                Err(error) => {
                    let error = error.to_string();
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "sftp.messages.action_failed",
                        &[("error", &error)],
                    )));
                    cx.notify();
                }
            }
            return;
        }

        let Some(commands) = self.tab(tab_id).and_then(|tab| tab.commands.clone()) else {
            return;
        };

        if let SftpPromptKind::ConfirmDelete {
            entries,
            refresh_path,
        } = prompt.kind
        {
            self.take_prompt(tab_id);
            cx.emit(AppCommand::OverlayDismissed(exit_snapshot));
            self.execute_delete(tab_id, &commands, entries, refresh_path, cx);
            return;
        }

        if let SftpPromptKind::ConfirmOverwrite {
            pending_uploads,
            pending_downloads,
            ..
        } = prompt.kind
        {
            self.take_prompt(tab_id);
            cx.emit(AppCommand::OverlayDismissed(exit_snapshot));
            for (local, remote) in pending_uploads {
                if let Err(error) = commands.queue_upload(local, remote) {
                    let error = error.to_string();
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "sftp.messages.upload_queue_failed",
                        &[("error", &error)],
                    )));
                    cx.notify();
                    return;
                }
            }
            for (remote, local) in pending_downloads {
                if let Err(error) = commands.queue_download(remote, local) {
                    let error = error.to_string();
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "sftp.messages.download_queue_failed",
                        &[("error", &error)],
                    )));
                    cx.notify();
                    return;
                }
            }
            cx.notify();
            return;
        }

        let value = self.prompt_input().read(cx).value().trim().to_string();
        if value.is_empty() {
            self.notify_validation_failure(
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("errors.sftp.validation.name_required"),
                cx,
            );
            return;
        }

        let result = match prompt.kind {
            SftpPromptKind::CreateRemoteDirectory { parent } => {
                let path = Self::join_remote_path(&parent, &value);
                let status_message = i18n::string_args(
                    "sftp.messages.creating_remote_directory",
                    &[("path", &path)],
                );
                commands
                    .create_directory(path)
                    .map(|_| (parent, status_message))
            }
            SftpPromptKind::ConfirmOverwrite { .. }
            | SftpPromptKind::ConfirmDelete { .. }
            | SftpPromptKind::ConfirmDeleteLocal { .. }
            | SftpPromptKind::CreateLocalDirectory { .. } => {
                unreachable!()
            }
        };

        match result {
            Ok((refresh_path, status_message)) => {
                self.take_prompt(tab_id);
                cx.emit(AppCommand::OverlayDismissed(exit_snapshot));
                cx.emit(AppCommand::Feedback(status_message));
                self.request_remote_directory(tab_id, refresh_path, cx);
            }
            Err(error) => {
                let error = error.to_string();
                cx.emit(AppCommand::Feedback(i18n::string_args(
                    "sftp.messages.action_failed",
                    &[("error", &error)],
                )));
                cx.notify();
            }
        }
    }

    pub(in crate::ui::shell) fn cancel_prompt(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(prompt) = self.take_prompt(tab_id) else {
            return;
        };
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::SftpPrompt { tab_id, prompt },
        ));
    }

    pub(in crate::ui::shell) fn join_remote_path(base: &str, name: &str) -> String {
        SftpService::join_remote_path(base, name)
    }

    pub(in crate::ui::shell) fn prompt(&self, tab_id: TabId) -> Option<SftpPromptState> {
        self.interactions
            .borrow()
            .get(&tab_id)
            .and_then(|state| state.prompt.clone())
    }

    pub(in crate::ui::shell) fn set_prompt(&self, tab_id: TabId, prompt: Option<SftpPromptState>) {
        self.interactions
            .borrow_mut()
            .entry(tab_id)
            .or_default()
            .prompt = prompt;
    }

    pub(in crate::ui::shell) fn take_prompt(&self, tab_id: TabId) -> Option<SftpPromptState> {
        self.interactions
            .borrow_mut()
            .get_mut(&tab_id)
            .and_then(|state| state.prompt.take())
    }

    pub(in crate::ui::shell) fn has_inline_rename(&self, tab_id: TabId) -> bool {
        self.interactions
            .borrow()
            .get(&tab_id)
            .is_some_and(|state| state.inline_rename.is_some())
    }

    fn inline_rename(&self, tab_id: TabId) -> Option<InlineRenameState> {
        self.interactions
            .borrow()
            .get(&tab_id)
            .and_then(|state| state.inline_rename.clone())
    }

    fn set_inline_rename(&self, tab_id: TabId, inline_rename: Option<InlineRenameState>) {
        self.interactions
            .borrow_mut()
            .entry(tab_id)
            .or_default()
            .inline_rename = inline_rename;
    }

    pub(in crate::ui::shell) fn edit_session_temp_path(
        &self,
        tab_id: TabId,
        remote_path: &str,
    ) -> Option<PathBuf> {
        self.interactions
            .borrow()
            .get(&tab_id)
            .and_then(|state| state.edit_sessions.get(remote_path))
            .map(|session| session.temp_path.clone())
    }

    pub(in crate::ui::shell) fn insert_edit_pending_download(
        &self,
        tab_id: TabId,
        transfer_id: TransferId,
        remote_path: String,
    ) {
        self.interactions
            .borrow_mut()
            .entry(tab_id)
            .or_default()
            .edit_pending_downloads
            .insert(transfer_id, remote_path);
    }

    pub(in crate::ui::shell) fn take_edit_pending_download(
        &self,
        tab_id: TabId,
        transfer_id: TransferId,
    ) -> Option<String> {
        self.interactions
            .borrow_mut()
            .get_mut(&tab_id)
            .and_then(|state| state.edit_pending_downloads.remove(&transfer_id))
    }

    pub(in crate::ui::shell) fn open_remote_file_for_editing(
        &mut self,
        tab_id: TabId,
        remote_path: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(temp_path) = self.edit_session_temp_path(tab_id, &remote_path) {
            if let Err(error) = open::that(temp_path) {
                let error = error.to_string();
                cx.emit(AppCommand::Feedback(i18n::string_args(
                    "sftp.messages.open_editor_failed",
                    &[("error", &error)],
                )));
                cx.notify();
            }
            return;
        }

        let Some(commands) = self.tab(tab_id).and_then(|tab| tab.commands.clone()) else {
            return;
        };
        let filename = std::path::Path::new(&remote_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        let temp_path = std::env::temp_dir()
            .join("miaominal_edit")
            .join(tab_id.to_string())
            .join(filename);

        if let Some(parent) = temp_path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            let error = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "sftp.messages.create_temp_directory_failed",
                &[("error", &error)],
            )));
            cx.notify();
            return;
        }

        match commands.queue_download(remote_path.clone(), temp_path) {
            Ok(transfer_id) => {
                self.insert_edit_pending_download(tab_id, transfer_id, remote_path);
                cx.notify();
            }
            Err(error) => {
                let error = error.to_string();
                cx.emit(AppCommand::Feedback(i18n::string_args(
                    "sftp.messages.queue_edit_download_failed",
                    &[("error", &error)],
                )));
                cx.notify();
            }
        }
    }

    pub(in crate::ui::shell) fn complete_edit_download(
        &mut self,
        tab_id: TabId,
        temp_path: PathBuf,
        remote_path: String,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = open::that(&temp_path) {
            let error = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "sftp.messages.open_editor_failed",
                &[("error", &error)],
            )));
            cx.notify();
            return;
        }

        if let Err(error) = self.start_edit_session(tab_id, temp_path, remote_path, cx) {
            let (key, error) = match error {
                SftpEditSessionStartError::CreateWatcher(error) => {
                    ("sftp.messages.create_file_watcher_failed", error)
                }
                SftpEditSessionStartError::WatchDirectory(error) => {
                    ("sftp.messages.watch_temp_directory_failed", error)
                }
            };
            cx.emit(AppCommand::Feedback(i18n::string_args(
                key,
                &[("error", &error)],
            )));
        }
        cx.notify();
    }

    fn start_edit_session(
        &mut self,
        tab_id: TabId,
        temp_path: PathBuf,
        remote_path: String,
        cx: &mut Context<Self>,
    ) -> Result<(), SftpEditSessionStartError> {
        let (sender, mut receiver) = futures::channel::mpsc::unbounded::<()>();
        let watch_path = temp_path.clone();
        let watch_sender = sender;
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                let Ok(event) = result else { return };
                let is_relevant = matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) && event.paths.iter().any(|path| path == &watch_path);
                if is_relevant {
                    let _ = watch_sender.unbounded_send(());
                }
            })
            .map_err(|error| SftpEditSessionStartError::CreateWatcher(error.to_string()))?;

        let watch_dir = temp_path
            .parent()
            .map(|path| path.to_path_buf())
            .unwrap_or_else(|| temp_path.clone());
        watcher
            .watch(&watch_dir, RecursiveMode::NonRecursive)
            .map_err(|error| SftpEditSessionStartError::WatchDirectory(error.to_string()))?;

        let remote_path_for_task = remote_path.clone();
        let temp_path_for_task = temp_path.clone();
        let watch_task = cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                let remote_path = remote_path_for_task.clone();
                let temp_path = temp_path_for_task.clone();
                if this
                    .update(cx, |this, cx| {
                        this.schedule_edit_upload_for_tab(tab_id, remote_path, temp_path, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let session = SftpEditSession {
            temp_path,
            _watcher: watcher,
            debounce_task: None,
            _watch_task: watch_task,
        };
        self.interactions
            .borrow_mut()
            .entry(tab_id)
            .or_default()
            .edit_sessions
            .insert(remote_path, session);
        Ok(())
    }

    fn schedule_edit_upload(
        &mut self,
        tab_id: TabId,
        remote_path: String,
        temp_path: PathBuf,
        commands: SftpCommandSender,
        cx: &mut Context<Self>,
    ) {
        {
            let mut interactions = self.interactions.borrow_mut();
            let Some(session) = interactions
                .get_mut(&tab_id)
                .and_then(|state| state.edit_sessions.get_mut(&remote_path))
            else {
                return;
            };
            session.debounce_task = None;
        }

        let remote_path_for_task = remote_path.clone();
        let debounce_task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            if let Err(error) = commands.queue_upload(temp_path, remote_path_for_task) {
                let error = error.to_string();
                let message =
                    i18n::string_args("sftp.messages.edit_upload_failed", &[("error", &error)]);
                let _ = this.update(cx, |this, cx| {
                    this.emit(AppCommand::Feedback(message), cx);
                });
            }
        });

        let mut interactions = self.interactions.borrow_mut();
        if let Some(session) = interactions
            .get_mut(&tab_id)
            .and_then(|state| state.edit_sessions.get_mut(&remote_path))
        {
            session.debounce_task = Some(debounce_task);
        }
    }

    fn schedule_edit_upload_for_tab(
        &mut self,
        tab_id: TabId,
        remote_path: String,
        temp_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(commands) = self.tab(tab_id).and_then(|tab| tab.commands.clone()) else {
            return;
        };
        self.schedule_edit_upload(tab_id, remote_path, temp_path, commands, cx);
    }

    pub(in crate::ui::shell) fn remove_tab_state(&self, tab_id: TabId) -> Option<SftpTabState> {
        self.interactions.borrow_mut().remove(&tab_id);
        self.tabs.borrow_mut().remove(tab_id)
    }

    pub(in crate::ui::shell) fn emit(&mut self, command: AppCommand, cx: &mut Context<Self>) {
        cx.emit(command);
    }

    pub(super) fn credentials_changed(
        &mut self,
        secrets: miaominal_secrets::SecretStore,
        cx: &mut Context<Self>,
    ) {
        self.service.replace_secrets(secrets);
        cx.notify();
    }
}

impl EventEmitter<AppCommand> for SftpController {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_delete_removes_files_and_directories_recursively() {
        let root = std::env::temp_dir().join(format!(
            "miaominal-sftp-local-delete-{}",
            uuid::Uuid::new_v4()
        ));
        let file = root.join("file.txt");
        let directory = root.join("directory");
        std::fs::create_dir_all(&directory).expect("create local delete test directory");
        std::fs::write(&file, b"file").expect("write local delete test file");
        std::fs::write(directory.join("nested.txt"), b"nested")
            .expect("write nested local delete test file");

        let result = delete_local_entries(vec![file.clone(), directory.clone()]);

        assert_eq!(result.deleted_count, 2);
        assert!(result.first_error.is_none());
        assert!(!file.exists());
        assert!(!directory.exists());
        let _ = std::fs::remove_dir(&root);
    }

    #[test]
    fn local_directory_creation_uses_the_current_local_directory() {
        let root = std::env::temp_dir().join(format!(
            "miaominal-sftp-local-create-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create local directory test root");

        let created = create_local_directory(&root, "new-folder")
            .expect("create folder inside current local directory");

        assert_eq!(created, root.join("new-folder"));
        assert!(created.is_dir());
        std::fs::remove_dir_all(&root).expect("clean local directory test root");
    }

    #[test]
    fn local_directory_name_must_be_a_single_path_component() {
        assert!(is_valid_local_directory_name("new-folder"));
        assert!(!is_valid_local_directory_name("."));
        assert!(!is_valid_local_directory_name(".."));
        assert!(!is_valid_local_directory_name("nested/folder"));
        assert!(!is_valid_local_directory_name("/absolute"));

        #[cfg(windows)]
        assert!(!is_valid_local_directory_name(r"nested\folder"));
    }

    #[test]
    fn remote_delete_normalization_prunes_duplicates_and_selected_descendants() {
        let entries = normalize_remote_delete_entries(vec![
            ("/root/child.txt".into(), false),
            ("/standalone.txt".into(), false),
            ("/root/nested".into(), true),
            ("/root".into(), true),
            ("/root/child.txt".into(), false),
        ]);

        assert_eq!(
            entries,
            vec![("/standalone.txt".into(), false), ("/root".into(), true),]
        );
    }

    #[test]
    fn remote_delete_normalization_uses_posix_component_boundaries() {
        let entries = normalize_remote_delete_entries(vec![
            ("/foo".into(), true),
            ("/foo/child.txt".into(), false),
            ("/foobar/child.txt".into(), false),
            ("/foo-bar/child.txt".into(), false),
        ]);

        assert_eq!(
            entries,
            vec![
                ("/foo".into(), true),
                ("/foobar/child.txt".into(), false),
                ("/foo-bar/child.txt".into(), false),
            ]
        );
    }

    fn transfer_row() -> SftpTransferRow {
        SftpTransferRow {
            transfer_id: TransferId(1),
            direction: TransferDirection::Upload,
            source: PathBuf::from("root"),
            destination: "/remote/root".to_string(),
            bytes_complete: 0,
            bytes_total: None,
            status: SftpTransferStatus::Queued,
            bytes_per_second: None,
            last_progress_at: None,
            last_bytes_complete: 0,
            is_directory: false,
            expanded: false,
            children: VecDeque::new(),
            child_count: 0,
        }
    }

    fn transfer_children() -> Vec<SftpTransferChild> {
        vec![
            SftpTransferChild {
                child_id: TransferChildId(0),
                relative_path: "done.txt".to_string(),
                bytes_total: Some(3),
            },
            SftpTransferChild {
                child_id: TransferChildId(1),
                relative_path: "active.txt".to_string(),
                bytes_total: Some(5),
            },
        ]
    }

    fn tab_state(profile_id: &str, tab_id: TabId) -> SftpTabState {
        let mut profile = miaominal_core::profile::SessionProfile::blank(profile_id, 1);
        profile.name = profile_id.to_string();
        profile.host = "example.com".into();
        let _ = tab_id;
        SftpTabState::new(&profile)
    }

    fn remote_entry(path: &str, kind: miaominal_sftp::SftpEntryKind) -> SftpEntry {
        SftpEntry {
            filename: SftpService::remote_file_name(path),
            path: path.to_string(),
            kind,
            size: None,
            modified: None,
            attributes: None,
            owner: None,
        }
    }

    #[test]
    fn single_file_save_uses_the_exact_native_dialog_path() {
        let entry = remote_entry("/remote/report.txt", miaominal_sftp::SftpEntryKind::File);
        let chosen = PathBuf::from("/chosen/custom-name.txt");

        let prepared = prepare_single_sftp_file_download(&entry, chosen.clone());

        assert!(prepared.overwrite_confirmed);
        assert_eq!(prepared.downloads.len(), 1);
        assert_eq!(prepared.downloads[0].remote_path, entry.path);
        assert_eq!(prepared.downloads[0].local_path, chosen);
    }

    #[test]
    fn single_file_dialog_is_reserved_for_confirmed_regular_files() {
        assert!(should_use_single_file_save_dialog(&[remote_entry(
            "/remote/report.txt",
            miaominal_sftp::SftpEntryKind::File,
        )]));
        assert!(!should_use_single_file_save_dialog(&[remote_entry(
            "/remote/link",
            miaominal_sftp::SftpEntryKind::Symlink,
        )]));
        assert!(!should_use_single_file_save_dialog(&[remote_entry(
            "/remote/folder",
            miaominal_sftp::SftpEntryKind::Directory,
        )]));
    }

    #[test]
    fn removed_tab_id_rejects_late_access_after_reopen() {
        let old_id = TabId::new(7);
        let reopened_id = TabId::new(8);
        let mut store = SftpTabStore::default();
        store.insert(old_id, tab_state("old-profile", old_id));

        let removed = store.remove(old_id).expect("old SFTP payload exists");
        assert_eq!(removed.profile_id, "old-profile");
        store.insert(reopened_id, tab_state("new-profile", reopened_id));

        assert!(store.tabs.get_mut(&old_id).is_none());
        assert_eq!(
            store
                .tabs
                .get(&reopened_id)
                .map(|tab| tab.profile_id.as_str()),
            Some("new-profile")
        );
    }

    #[test]
    fn standalone_reopen_uses_saved_profile_after_catalog_removal() {
        let mut saved = SessionProfile::blank("removed-profile", 1);
        saved.host = "removed.example.com".into();

        let resolved = SftpController::resolve_start_profile(saved.clone(), None, None)
            .expect("standalone SFTP reopen should use its saved profile");

        assert_eq!(resolved.id, saved.id);
        assert_eq!(resolved.host, saved.host);
    }

    #[test]
    fn session_sidecar_still_requires_its_active_owner_profile() {
        let saved = SessionProfile::blank("profile", 1);

        let error = SftpController::resolve_start_profile(saved, Some(TabId::new(7)), None)
            .expect_err("session sidecar should require an active owner profile");

        assert!(error.contains("7"));
    }

    #[test]
    fn progress_center_state_can_open_without_an_sftp_tab() {
        let mut visible = false;
        let mut transition = None;

        assert!(update_progress_center_state(
            &mut visible,
            &mut transition,
            true,
            Instant::now(),
        ));
        assert!(visible);
        assert!(matches!(
            transition,
            Some(SftpProgressCenterTransition {
                phase: SftpProgressCenterTransitionPhase::Entering,
                ..
            })
        ));
    }

    #[test]
    fn transfer_refresh_uses_destination_parent_directory() {
        assert_eq!(
            SftpController::transfer_remote_parent_path("/file.txt"),
            "/"
        );
        assert_eq!(
            SftpController::transfer_remote_parent_path("/remote/file.txt"),
            "/remote"
        );
        assert_eq!(
            SftpController::transfer_remote_parent_path("relative.txt"),
            "."
        );
    }

    #[test]
    fn directory_response_must_match_the_latest_request() {
        let first = SftpDirectoryRequestId(1);
        let second = SftpDirectoryRequestId(2);

        assert!(SftpController::is_current_directory_response(
            Some(second),
            Some(second),
        ));
        assert!(!SftpController::is_current_directory_response(
            Some(second),
            Some(first),
        ));
        assert!(SftpController::is_current_directory_response(None, None));
    }

    #[test]
    fn pending_navigation_blocks_refresh_of_the_loaded_directory() {
        let pending = HashSet::from(["/loaded".to_string(), "/next".to_string()]);

        assert_eq!(
            SftpController::debounced_remote_refresh_target(&pending, "/loaded", Some("/next"),),
            Some("/next".to_string())
        );
        assert_eq!(
            SftpController::debounced_remote_refresh_target(
                &HashSet::from(["/loaded".to_string()]),
                "/loaded",
                Some("/next"),
            ),
            None
        );
    }

    #[test]
    fn transfer_child_updates_and_failure_preserve_completed_files() {
        let mut transfer = transfer_row();
        let mut children = transfer_children().into_iter();
        transfer.push_child(children.next().expect("completed child"));

        assert!(transfer.is_directory);
        assert!(!transfer.expanded);
        assert_eq!(transfer.children.len(), 1);

        transfer.apply_child_update(SftpTransferChildUpdate {
            child_id: TransferChildId(0),
            bytes_complete: 3,
            state: SftpTransferChildState::Done,
        });
        transfer.push_child(children.next().expect("active child"));
        transfer.apply_child_update(SftpTransferChildUpdate {
            child_id: TransferChildId(1),
            bytes_complete: 2,
            state: SftpTransferChildState::Running,
        });
        transfer.pause_active_child();
        assert!(matches!(
            &transfer.children[1].status,
            SftpTransferChildStatus::Paused
        ));
        transfer.resume_active_child();
        assert!(matches!(
            &transfer.children[1].status,
            SftpTransferChildStatus::Running
        ));

        transfer.fail_unfinished_children("boom");

        assert!(matches!(
            &transfer.children[0].status,
            SftpTransferChildStatus::Done
        ));
        assert!(matches!(
            &transfer.children[1].status,
            SftpTransferChildStatus::Failed(message) if message == "boom"
        ));
    }

    #[test]
    fn transfer_cancel_marks_only_unfinished_children() {
        let mut transfer = transfer_row();
        let mut children = transfer_children().into_iter();
        transfer.push_child(children.next().expect("completed child"));
        transfer.apply_child_update(SftpTransferChildUpdate {
            child_id: TransferChildId(0),
            bytes_complete: 3,
            state: SftpTransferChildState::Done,
        });
        transfer.push_child(children.next().expect("active child"));
        transfer.apply_child_update(SftpTransferChildUpdate {
            child_id: TransferChildId(1),
            bytes_complete: 2,
            state: SftpTransferChildState::Running,
        });

        transfer.cancel_unfinished_children();

        assert!(matches!(
            &transfer.children[0].status,
            SftpTransferChildStatus::Done
        ));
        assert!(matches!(
            &transfer.children[1].status,
            SftpTransferChildStatus::Cancelled
        ));
    }

    #[test]
    fn delayed_running_progress_does_not_regress_completed_child() {
        let mut transfer = transfer_row();
        transfer.push_child(transfer_children().remove(0));
        transfer.apply_child_update(SftpTransferChildUpdate {
            child_id: TransferChildId(0),
            bytes_complete: 3,
            state: SftpTransferChildState::Done,
        });

        transfer.apply_child_update(SftpTransferChildUpdate {
            child_id: TransferChildId(0),
            bytes_complete: 2,
            state: SftpTransferChildState::Running,
        });

        assert_eq!(transfer.children[0].bytes_complete, 3);
        assert!(matches!(
            transfer.children[0].status,
            SftpTransferChildStatus::Done
        ));
    }

    #[test]
    fn transfer_child_history_is_bounded() {
        let mut transfer = transfer_row();
        let total = SFTP_TRANSFER_CHILD_HISTORY_LIMIT + 7;
        for index in 0..total {
            let child_id = TransferChildId(index as u64);
            transfer.push_child(SftpTransferChild {
                child_id,
                relative_path: format!("file-{index}.txt"),
                bytes_total: Some(1),
            });
            transfer.apply_child_update(SftpTransferChildUpdate {
                child_id,
                bytes_complete: 1,
                state: SftpTransferChildState::Done,
            });
        }

        assert_eq!(transfer.children.len(), SFTP_TRANSFER_CHILD_HISTORY_LIMIT);
        assert_eq!(transfer.child_count, total as u64);
        assert_eq!(transfer.omitted_child_count(), 7);
        assert_eq!(
            transfer.children.front().map(|child| child.child_id),
            Some(TransferChildId(7))
        );
    }
}
