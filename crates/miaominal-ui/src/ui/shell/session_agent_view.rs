//! Persistent, independently invalidated views for the session-agent conversation.
//!
//! This module intentionally keeps the data-to-view projection small. The existing
//! conversation renderer reads visible message entities by index to keep its complete
//! message/tool UI while gaining `gpui::list` virtualization. The `Render`
//! implementations below are useful defaults and keep Markdown state persistent, but
//! they are not intended to duplicate every AppView-owned action.

use super::{SessionAgentMessage, SessionAgentMessageRole};
use crate::ui::shell::support::LIST_ENTER_DURATION;
use crate::ui::{components::md3_spinner, i18n};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EntityId, EventEmitter, FocusHandle,
    Focusable, FollowMode, FontWeight, InteractiveElement as _, IntoElement, ListAlignment,
    ListOffset, ListState, MouseButton, ParentElement as _, Pixels, Render, SharedString,
    Styled as _, Subscription, Task, Window, div, list, px, rgb,
};
use gpui_component::{h_flex, text::TextView, text::TextViewState, v_flex};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    time::{Duration, Instant},
};

const SESSION_AGENT_LIST_OVERDRAW: f32 = 2048.0;
const THINKING_TICK_INTERVAL: Duration = Duration::from_secs(1);

fn restore_list_tail_follow_mode(list_state: &ListState) {
    let offset = list_state.logical_scroll_top();
    list_state.scroll_to(offset);
    list_state.set_follow_mode(FollowMode::Tail);
}

fn message_can_start_enter_motion(message: &SessionAgentMessage) -> bool {
    message.role != SessionAgentMessageRole::Assistant || !message.content.trim().is_empty()
}

/// Describes how an authoritative content snapshot changed a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SessionAgentContentUpdate {
    Unchanged,
    Appended { bytes: usize, chars: usize },
    Replaced,
}

impl SessionAgentContentUpdate {
    pub(in crate::ui::shell) fn changed(self) -> bool {
        !matches!(self, Self::Unchanged)
    }
}

/// Events emitted by a message view for consumers that cache its measured height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SessionAgentMessageViewEvent {
    ContentUpdated(SessionAgentContentUpdate),
    LayoutChanged,
    Refresh,
}

#[derive(Clone, Copy)]
enum SessionAgentEnterMotionState {
    Absent,
    Pending { key: u64 },
    Playing { key: u64, started_at: Instant },
    Consumed { key: u64 },
}

impl SessionAgentEnterMotionState {
    fn new(key: Option<u64>) -> Self {
        key.map_or(Self::Absent, |key| Self::Pending { key })
    }

    fn key(self) -> Option<u64> {
        match self {
            Self::Absent => None,
            Self::Pending { key } | Self::Playing { key, .. } | Self::Consumed { key } => Some(key),
        }
    }

    fn sync(&mut self, key: Option<u64>) {
        if self.key() != key {
            *self = Self::new(key);
        }
    }

    fn active_key(&mut self) -> Option<u64> {
        match *self {
            Self::Absent | Self::Consumed { .. } => None,
            Self::Pending { key } => {
                *self = Self::Playing {
                    key,
                    started_at: Instant::now(),
                };
                Some(key)
            }
            Self::Playing { key, started_at } => {
                if started_at.elapsed() < LIST_ENTER_DURATION {
                    Some(key)
                } else {
                    *self = Self::Consumed { key };
                    None
                }
            }
        }
    }

    fn active_key_when(&mut self, enabled: bool) -> Option<u64> {
        if enabled { self.active_key() } else { None }
    }

    /// Keeps only animations that have never started. Playing/consumed messages must not
    /// restart when a hidden conversation view is released and lazily rebuilt later.
    fn key_for_rebuild(self) -> Option<u64> {
        match self {
            Self::Pending { key } => Some(key),
            Self::Absent | Self::Playing { .. } | Self::Consumed { .. } => None,
        }
    }
}

/// Persistent UI projection for one [`SessionAgentMessage`].
///
/// The Markdown entity is created only when the row is first rendered (or explicitly
/// requested through [`Self::ensure_markdown_state`]). Streaming updates append to the
/// entity in place. Its asynchronous parse notifications are forwarded as
/// [`SessionAgentMessageViewEvent::LayoutChanged`] so an outer virtual list can remeasure
/// only this row.
pub(in crate::ui::shell) struct SessionAgentMessageView {
    message: SessionAgentMessage,
    focus_handle: FocusHandle,
    enter_motion: SessionAgentEnterMotionState,
    markdown_state: Option<Entity<TextViewState>>,
    markdown_observation: Option<Subscription>,
    markdown_worker_seeded: bool,
    thinking_ticker: Option<Task<()>>,
    content_chars: usize,
    estimated_tokens: usize,
    selectable: bool,
}

impl EventEmitter<SessionAgentMessageViewEvent> for SessionAgentMessageView {}

impl Focusable for SessionAgentMessageView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[allow(dead_code)]
impl SessionAgentMessageView {
    pub(in crate::ui::shell) fn new(message: SessionAgentMessage, cx: &mut Context<Self>) -> Self {
        let content_chars = message.content.chars().count();
        let enter_motion = SessionAgentEnterMotionState::new(message.motion.enter_key);
        let mut this = Self {
            message,
            focus_handle: cx.focus_handle(),
            enter_motion,
            markdown_state: None,
            markdown_observation: None,
            markdown_worker_seeded: false,
            thinking_ticker: None,
            content_chars,
            estimated_tokens: estimate_tokens_from_chars(content_chars),
            selectable: true,
        };
        this.refresh_thinking_ticker(cx);
        this
    }

    pub(in crate::ui::shell) fn message(&self) -> &SessionAgentMessage {
        &self.message
    }

    /// Produces an authoritative DTO snapshot for persistence, request history, and titles.
    pub(in crate::ui::shell) fn snapshot(&self) -> SessionAgentMessage {
        self.message.clone()
    }

    /// Produces the metadata needed by a visible row. Normal Markdown rendering reads from the
    /// persistent `TextViewState`, so the potentially large source string only needs to be copied
    /// while conversation search is active.
    pub(in crate::ui::shell) fn render_snapshot(
        &self,
        include_content: bool,
    ) -> SessionAgentMessage {
        SessionAgentMessage {
            role: self.message.role,
            content: include_content
                .then(|| self.message.content.clone())
                .unwrap_or_default(),
            tool_call: self.message.tool_call.clone(),
            thinking: self.message.thinking.clone(),
            motion: Default::default(),
            attachments: self.message.attachments.clone(),
        }
    }

    pub(in crate::ui::shell) fn active_enter_motion_key(&mut self) -> Option<u64> {
        self.enter_motion
            .active_key_when(message_can_start_enter_motion(&self.message))
    }

    pub(in crate::ui::shell) fn enter_motion_key_for_rebuild(&self) -> Option<u64> {
        self.enter_motion.key_for_rebuild()
    }

    pub(in crate::ui::shell) fn role(&self) -> SessionAgentMessageRole {
        self.message.role
    }

    pub(in crate::ui::shell) fn content(&self) -> &str {
        &self.message.content
    }

    pub(in crate::ui::shell) fn content_chars(&self) -> usize {
        self.content_chars
    }

    pub(in crate::ui::shell) fn estimated_tokens(&self) -> usize {
        self.estimated_tokens
    }

    pub(in crate::ui::shell) fn markdown_state(&self) -> Option<Entity<TextViewState>> {
        self.markdown_state.clone()
    }

    pub(in crate::ui::shell) fn set_selectable(
        &mut self,
        selectable: bool,
        cx: &mut Context<Self>,
    ) {
        if self.selectable == selectable {
            return;
        }

        self.selectable = selectable;
        if let Some(state) = self.markdown_state.as_ref() {
            state.update(cx, |state, cx| state.set_selectable(selectable, cx));
        }
        cx.notify();
    }

    /// Lazily creates and returns the persistent Markdown state for this row.
    pub(in crate::ui::shell) fn ensure_markdown_state(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Entity<TextViewState> {
        if let Some(state) = self.markdown_state.as_ref() {
            return state.clone();
        }

        self.rebuild_markdown_state(cx)
    }

    /// The pinned gpui-component revision parses full replacements synchronously, but its
    /// background append worker does not inherit that parsed baseline. Keep the correct
    /// synchronous first layout here and lazily seed the worker only if this row later receives
    /// an append; static history rows therefore pay for just one full parse.
    fn rebuild_markdown_state(&mut self, cx: &mut Context<Self>) -> Entity<TextViewState> {
        let source = self.message.content.clone();
        self.markdown_worker_seeded = source.is_empty();
        let selectable = self.selectable;
        let state = cx.new(move |cx| TextViewState::markdown(&source, cx).selectable(selectable));
        self.markdown_observation = Some(cx.observe(&state, |_this, _state, cx| {
            cx.emit(SessionAgentMessageViewEvent::LayoutChanged);
            cx.notify();
        }));
        self.markdown_state = Some(state.clone());
        state
    }

    /// Appends to the managed Markdown document, seeding gpui-component's background worker on
    /// the first real append after a non-empty synchronous full parse. `push_str(full_source)`
    /// gives the empty worker its baseline; the immediately newer `set_text(full_source)` keeps
    /// the visible entity correct and makes the worker result stale. Later suffixes can use the
    /// normal append path.
    fn append_markdown_suffix(&mut self, suffix: &str, cx: &mut Context<Self>) {
        let Some(state) = self.markdown_state.clone() else {
            return;
        };

        if self.markdown_worker_seeded {
            state.update(cx, |state, cx| state.push_str(suffix, cx));
            return;
        }

        let source = self.message.content.clone();
        state.update(cx, |state, cx| {
            state.push_str(&source, cx);
            state.set_text(&source, cx);
        });
        self.markdown_worker_seeded = true;
    }

    /// Appends a streaming suffix; after the one-time worker seed, steady-state updates use
    /// gpui-component's append path instead of replacing the full source.
    pub(in crate::ui::shell) fn append_content(
        &mut self,
        suffix: &str,
        cx: &mut Context<Self>,
    ) -> SessionAgentContentUpdate {
        if suffix.is_empty() {
            return SessionAgentContentUpdate::Unchanged;
        }

        let chars = suffix.chars().count();
        self.message.content.push_str(suffix);
        self.content_chars = self.content_chars.saturating_add(chars);
        self.estimated_tokens = estimate_tokens_from_chars(self.content_chars);

        self.append_markdown_suffix(suffix, cx);

        let update = SessionAgentContentUpdate::Appended {
            bytes: suffix.len(),
            chars,
        };
        cx.emit(SessionAgentMessageViewEvent::ContentUpdated(update));
        cx.notify();
        update
    }

    /// Applies an authoritative content snapshot using append when it extends the current text.
    pub(in crate::ui::shell) fn set_content_snapshot(
        &mut self,
        snapshot: impl Into<String>,
        cx: &mut Context<Self>,
    ) -> SessionAgentContentUpdate {
        let snapshot = snapshot.into();
        if snapshot == self.message.content {
            return SessionAgentContentUpdate::Unchanged;
        }

        if snapshot.starts_with(&self.message.content) {
            let suffix = &snapshot[self.message.content.len()..];
            return self.append_content(suffix, cx);
        }

        self.message.content = snapshot;
        self.recount_content();
        if self.markdown_state.is_some() {
            self.rebuild_markdown_state(cx);
        }

        let update = SessionAgentContentUpdate::Replaced;
        cx.emit(SessionAgentMessageViewEvent::ContentUpdated(update));
        cx.notify();
        update
    }

    /// Replaces the complete data snapshot while retaining the persistent message projection.
    pub(in crate::ui::shell) fn set_message_snapshot(
        &mut self,
        message: SessionAgentMessage,
        cx: &mut Context<Self>,
    ) -> SessionAgentContentUpdate {
        let old_content = std::mem::replace(&mut self.message, message).content;
        self.enter_motion.sync(self.message.motion.enter_key);
        let update = if self.message.content == old_content {
            SessionAgentContentUpdate::Unchanged
        } else if self.message.content.starts_with(&old_content) {
            let suffix = self.message.content[old_content.len()..].to_string();
            let chars = suffix.chars().count();
            self.append_markdown_suffix(&suffix, cx);
            SessionAgentContentUpdate::Appended {
                bytes: suffix.len(),
                chars,
            }
        } else {
            if self.markdown_state.is_some() {
                self.rebuild_markdown_state(cx);
            }
            SessionAgentContentUpdate::Replaced
        };

        self.recount_content();
        self.refresh_thinking_ticker(cx);
        if update.changed() {
            cx.emit(SessionAgentMessageViewEvent::ContentUpdated(update));
        } else {
            cx.emit(SessionAgentMessageViewEvent::LayoutChanged);
        }
        cx.notify();
        update
    }

    /// Mutates non-streaming message metadata and keeps the Markdown state synchronized if
    /// the callback also changes the content. Prefer [`Self::append_content`] on the hot path.
    pub(in crate::ui::shell) fn update_message(
        &mut self,
        update: impl FnOnce(&mut SessionAgentMessage),
        cx: &mut Context<Self>,
    ) -> SessionAgentContentUpdate {
        let old_content = self.message.content.clone();
        update(&mut self.message);
        self.enter_motion.sync(self.message.motion.enter_key);

        let content_update = if self.message.content == old_content {
            SessionAgentContentUpdate::Unchanged
        } else if self.message.content.starts_with(&old_content) {
            let suffix = self.message.content[old_content.len()..].to_string();
            let chars = suffix.chars().count();
            self.append_markdown_suffix(&suffix, cx);
            SessionAgentContentUpdate::Appended {
                bytes: suffix.len(),
                chars,
            }
        } else {
            if self.markdown_state.is_some() {
                self.rebuild_markdown_state(cx);
            }
            SessionAgentContentUpdate::Replaced
        };

        self.recount_content();
        self.refresh_thinking_ticker(cx);
        if content_update.changed() {
            cx.emit(SessionAgentMessageViewEvent::ContentUpdated(content_update));
        } else {
            cx.emit(SessionAgentMessageViewEvent::LayoutChanged);
        }
        cx.notify();
        content_update
    }

    pub(in crate::ui::shell) fn set_thinking_expanded(
        &mut self,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(thinking) = self.message.thinking.as_mut() else {
            return;
        };
        if thinking.expanded == expanded {
            return;
        }

        thinking.expanded = expanded;
        cx.emit(SessionAgentMessageViewEvent::LayoutChanged);
        cx.notify();
    }

    pub(in crate::ui::shell) fn toggle_thinking_expanded(&mut self, cx: &mut Context<Self>) {
        let expanded = self
            .message
            .thinking
            .as_ref()
            .is_some_and(|thinking| thinking.expanded);
        self.set_thinking_expanded(!expanded, cx);
    }

    pub(in crate::ui::shell) fn finish_thinking(&mut self, cx: &mut Context<Self>) {
        if let Some(thinking) = self.message.thinking.as_mut() {
            if thinking.elapsed_ms.is_none() {
                thinking.elapsed_ms = Some(thinking.started_at.elapsed().as_millis());
            }
        }
        self.thinking_ticker = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn elapsed_thinking_ms(&self) -> Option<u128> {
        self.message.thinking.as_ref().map(|thinking| {
            thinking
                .elapsed_ms
                .unwrap_or_else(|| thinking.started_at.elapsed().as_millis())
        })
    }

    fn recount_content(&mut self) {
        self.content_chars = self.message.content.chars().count();
        self.estimated_tokens = estimate_tokens_from_chars(self.content_chars);
    }

    fn is_active_thinking(&self) -> bool {
        self.message.role == SessionAgentMessageRole::Thinking
            && self
                .message
                .thinking
                .as_ref()
                .is_some_and(|thinking| thinking.elapsed_ms.is_none())
    }

    fn refresh_thinking_ticker(&mut self, cx: &mut Context<Self>) {
        self.thinking_ticker = None;
        if !self.is_active_thinking() {
            return;
        }

        self.thinking_ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(THINKING_TICK_INTERVAL).await;
                let keep_ticking = this
                    .update(cx, |this, cx| {
                        if this.is_active_thinking() {
                            // This invalidates only the message entity. It intentionally does not
                            // emit LayoutChanged, since the elapsed label has stable row height.
                            cx.emit(SessionAgentMessageViewEvent::Refresh);
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if !keep_ticking {
                    break;
                }
            }
        }));
    }

    fn render_markdown(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.message.content.trim().is_empty() {
            return div().into_any_element();
        }

        let state = self.ensure_markdown_state(cx);
        TextView::new(&state)
            .selectable(self.selectable)
            .w_full()
            .min_w_0()
            .into_any_element()
    }
}

impl Render for SessionAgentMessageView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;
        let text_size = miaominal_settings::FontSize::Input.scaled();
        let role = self.message.role;

        if role == SessionAgentMessageRole::Thinking {
            let expanded = self
                .message
                .thinking
                .as_ref()
                .is_some_and(|thinking| thinking.expanded);
            let active = self.is_active_thinking();
            let elapsed = self.elapsed_thinking_ms().unwrap_or_default();
            let token_count = self.estimated_tokens;
            let entity = cx.entity();
            let markdown = expanded.then(|| self.render_markdown(cx));

            return v_flex()
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .gap_1()
                .px_3()
                .py_1()
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .items_center()
                        .cursor_pointer()
                        .text_size(text_size)
                        .text_color(rgb(roles.on_surface_variant))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            entity.update(cx, |this, cx| this.toggle_thinking_expanded(cx));
                        })
                        .child(if expanded { "\u{25be}" } else { "\u{25b8}" })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(i18n::string("workspace.panel.agent.thinking_title")),
                        )
                        .when(active, |this| {
                            this.child(format_duration_ms(elapsed))
                                .child(format!("~{token_count} tok"))
                        }),
                )
                .when_some(markdown, |this, markdown| {
                    this.child(div().w_full().min_w_0().pl_5().child(markdown))
                })
                .into_any_element();
        }

        let markdown = self.render_markdown(cx);
        let is_user = role == SessionAgentMessageRole::User;
        let is_error = role == SessionAgentMessageRole::Error;
        let is_tool = role == SessionAgentMessageRole::ToolCall;

        let background = if is_user {
            roles.primary_container
        } else if is_error {
            roles.error_container
        } else if is_tool {
            roles.surface_container_high
        } else {
            roles.surface
        };
        let foreground = if is_user {
            roles.on_primary_container
        } else if is_error {
            roles.on_error_container
        } else {
            roles.on_surface
        };

        h_flex()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .px_1()
            .py_1()
            .when(is_user, |this| this.justify_end())
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .rounded(px(8.0))
                    .when(is_user || is_error || is_tool, |this| {
                        this.bg(rgb(background)).px_3().py_2()
                    })
                    .text_color(rgb(foreground))
                    .when(is_error, |this| {
                        this.gap_1().child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(text_size)
                                .child(i18n::string("workspace.panel.agent.error")),
                        )
                    })
                    .child(markdown),
            )
            .into_any_element()
    }
}

/// Persistent virtual-list container for one session-agent conversation.
pub(in crate::ui::shell) struct SessionAgentConversationView {
    messages: Vec<Entity<SessionAgentMessageView>>,
    message_indices: HashMap<EntityId, usize>,
    message_subscriptions: Vec<Subscription>,
    /// `TextViewState` uses the same untyped notification for parsed Markdown, selection,
    /// hover, and focus-related changes. While the user is dragging a selection, defer the
    /// resulting list remeasurements and coalesce them by message entity. The TextView itself
    /// still receives its notification and can repaint the selection immediately.
    selection_drag_active: bool,
    deferred_remeasure: HashSet<EntityId>,
    list_state: ListState,
    generating: bool,
    generating_label: SharedString,
    generating_view: Entity<SessionAgentGeneratingView>,
}

pub(in crate::ui::shell) struct SessionAgentGeneratingView {
    label: SharedString,
}

impl Render for SessionAgentGeneratingView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let material = miaominal_settings::current_theme().material;
        let text_muted = crate::ui::theme::palette_tone_rgb(
            material.palettes.neutral_variant,
            if material.dark { 65 } else { 50 },
        );
        h_flex()
            .w_full()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .text_size(miaominal_settings::FontSize::Input.scaled())
            .text_color(rgb(text_muted))
            .child(md3_spinner(16.0))
            .child(self.label.clone())
    }
}

#[allow(dead_code)]
impl SessionAgentConversationView {
    pub(in crate::ui::shell) fn from_messages(
        messages: Vec<SessionAgentMessage>,
        generating: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let messages = messages
            .into_iter()
            .map(|message| cx.new(move |cx| SessionAgentMessageView::new(message, cx)))
            .collect();
        Self::new(messages, generating, cx)
    }

    pub(in crate::ui::shell) fn new(
        messages: Vec<Entity<SessionAgentMessageView>>,
        generating: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let list_state = ListState::new(0, ListAlignment::Top, px(SESSION_AGENT_LIST_OVERDRAW));
        list_state.splice_focusable(
            0..0,
            messages
                .iter()
                .map(|message| Some(message.focus_handle(cx)))
                .chain(generating.then_some(None)),
        );
        list_state.set_follow_mode(FollowMode::Tail);

        let message_subscriptions = messages
            .iter()
            .map(|message| Self::subscribe_to_message(message, cx))
            .collect();
        let message_indices = messages
            .iter()
            .enumerate()
            .map(|(index, message)| (message.entity_id(), index))
            .collect();

        let generating_label: SharedString = i18n::string("workspace.panel.agent.thinking").into();
        let generating_view = cx.new({
            let label = generating_label.clone();
            move |_| SessionAgentGeneratingView { label }
        });

        Self {
            messages,
            message_indices,
            message_subscriptions,
            selection_drag_active: false,
            deferred_remeasure: HashSet::new(),
            list_state,
            generating,
            generating_label,
            generating_view,
        }
    }

    pub(in crate::ui::shell) fn messages(&self) -> &[Entity<SessionAgentMessageView>] {
        &self.messages
    }

    pub(in crate::ui::shell) fn message(
        &self,
        index: usize,
    ) -> Option<Entity<SessionAgentMessageView>> {
        self.messages.get(index).cloned()
    }

    pub(in crate::ui::shell) fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub(in crate::ui::shell) fn message_snapshots(&self, cx: &App) -> Vec<SessionAgentMessage> {
        self.messages
            .iter()
            .map(|message| message.read(cx).snapshot())
            .collect()
    }

    pub(in crate::ui::shell) fn enter_motion_keys_for_rebuild(&self, cx: &App) -> Vec<Option<u64>> {
        self.messages
            .iter()
            .map(|message| message.read(cx).enter_motion_key_for_rebuild())
            .collect()
    }

    pub(in crate::ui::shell) fn list_state(&self) -> ListState {
        self.list_state.clone()
    }

    pub(in crate::ui::shell) fn is_generating(&self) -> bool {
        self.generating
    }

    pub(in crate::ui::shell) fn generating_view(&self) -> Entity<SessionAgentGeneratingView> {
        self.generating_view.clone()
    }

    pub(in crate::ui::shell) fn push_message(
        &mut self,
        message: SessionAgentMessage,
        cx: &mut Context<Self>,
    ) -> Entity<SessionAgentMessageView> {
        let entity = cx.new(move |cx| SessionAgentMessageView::new(message, cx));
        self.push_message_entity(entity.clone(), cx);
        entity
    }

    pub(in crate::ui::shell) fn push_message_entity(
        &mut self,
        message: Entity<SessionAgentMessageView>,
        cx: &mut Context<Self>,
    ) {
        let index = self.messages.len();
        let focus_handle = message.focus_handle(cx);
        self.message_subscriptions
            .push(Self::subscribe_to_message(&message, cx));
        self.message_indices.insert(message.entity_id(), index);
        self.messages.push(message);
        self.list_state
            .splice_focusable(index..index, [Some(focus_handle)]);
        cx.notify();
    }

    pub(in crate::ui::shell) fn insert_message_entity(
        &mut self,
        index: usize,
        message: Entity<SessionAgentMessageView>,
        cx: &mut Context<Self>,
    ) -> bool {
        if index > self.messages.len() {
            return false;
        }

        let focus_handle = message.focus_handle(cx);
        let subscription = Self::subscribe_to_message(&message, cx);
        self.messages.insert(index, message);
        self.message_subscriptions.insert(index, subscription);
        self.rebuild_message_indices();
        self.list_state
            .splice_focusable(index..index, [Some(focus_handle)]);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn replace_message_entity(
        &mut self,
        index: usize,
        message: Entity<SessionAgentMessageView>,
        cx: &mut Context<Self>,
    ) -> Option<Entity<SessionAgentMessageView>> {
        if index >= self.messages.len() {
            return None;
        }

        let focus_handle = message.focus_handle(cx);
        let subscription = Self::subscribe_to_message(&message, cx);
        let previous = std::mem::replace(&mut self.messages[index], message);
        self.message_subscriptions[index] = subscription;
        self.rebuild_message_indices();
        self.list_state
            .splice_focusable(index..index + 1, [Some(focus_handle)]);
        cx.notify();
        Some(previous)
    }

    pub(in crate::ui::shell) fn remove_message(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<Entity<SessionAgentMessageView>> {
        if index >= self.messages.len() {
            return None;
        }

        let _ = self.message_subscriptions.remove(index);
        let message = self.messages.remove(index);
        self.rebuild_message_indices();
        self.list_state.splice(index..index + 1, 0);
        cx.notify();
        Some(message)
    }

    pub(in crate::ui::shell) fn replace_all(
        &mut self,
        messages: Vec<Entity<SessionAgentMessageView>>,
        cx: &mut Context<Self>,
    ) {
        let old_len = self.messages.len();
        let focus_handles = messages
            .iter()
            .map(|message| Some(message.focus_handle(cx)))
            .collect::<Vec<_>>();
        self.message_subscriptions = messages
            .iter()
            .map(|message| Self::subscribe_to_message(message, cx))
            .collect();
        self.messages = messages;
        self.rebuild_message_indices();
        self.list_state.splice_focusable(0..old_len, focus_handles);
        cx.notify();
    }

    pub(in crate::ui::shell) fn clear(&mut self, cx: &mut Context<Self>) {
        let old_len = self.messages.len();
        if old_len == 0 {
            return;
        }

        self.messages.clear();
        self.message_indices.clear();
        self.message_subscriptions.clear();
        self.list_state.splice(0..old_len, 0);
        cx.notify();
    }

    pub(in crate::ui::shell) fn append_to_message(
        &mut self,
        index: usize,
        suffix: &str,
        cx: &mut Context<Self>,
    ) -> SessionAgentContentUpdate {
        let Some(message) = self.messages.get(index).cloned() else {
            return SessionAgentContentUpdate::Unchanged;
        };
        message.update(cx, |message, cx| message.append_content(suffix, cx))
    }

    pub(in crate::ui::shell) fn set_message_content_snapshot(
        &mut self,
        index: usize,
        snapshot: impl Into<String>,
        cx: &mut Context<Self>,
    ) -> SessionAgentContentUpdate {
        let Some(message) = self.messages.get(index).cloned() else {
            return SessionAgentContentUpdate::Unchanged;
        };
        let snapshot = snapshot.into();
        message.update(cx, |message, cx| message.set_content_snapshot(snapshot, cx))
    }

    pub(in crate::ui::shell) fn set_message_snapshot(
        &mut self,
        index: usize,
        snapshot: SessionAgentMessage,
        cx: &mut Context<Self>,
    ) -> SessionAgentContentUpdate {
        let Some(message) = self.messages.get(index).cloned() else {
            return SessionAgentContentUpdate::Unchanged;
        };
        message.update(cx, |message, cx| message.set_message_snapshot(snapshot, cx))
    }

    pub(in crate::ui::shell) fn set_generating(
        &mut self,
        generating: bool,
        cx: &mut Context<Self>,
    ) {
        if self.generating == generating {
            return;
        }

        let index = self.messages.len();
        if generating {
            self.list_state.splice(index..index, 1);
        } else {
            self.list_state.splice(index..index + 1, 0);
        }
        self.generating = generating;
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_generating_label(
        &mut self,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let label = label.into();
        if !replace_generating_label_if_changed(&mut self.generating_label, label) {
            return;
        }
        let label = self.generating_label.clone();
        self.generating_view.update(cx, |view, cx| {
            view.label = label;
            cx.notify();
        });
        // The generating label is read from the current locale whenever the persistent
        // conversation view is ensured. Treat an actual label change as a locale-change
        // signal: message rows also render localized labels, including rows currently
        // outside the virtual-list viewport, so every cached height must be invalidated.
        self.list_state.remeasure();
        cx.notify();
    }

    pub(in crate::ui::shell) fn remeasure_message(&self, index: usize) {
        if index < self.messages.len() {
            self.list_state.remeasure_items(index..index + 1);
        }
    }

    pub(in crate::ui::shell) fn remeasure_messages(&self, range: Range<usize>) {
        let start = range.start.min(self.messages.len());
        let end = range.end.min(self.messages.len());
        if start < end {
            self.list_state.remeasure_items(start..end);
        }
    }

    pub(in crate::ui::shell) fn remeasure_all(&self) {
        self.list_state.remeasure();
    }

    pub(in crate::ui::shell) fn scroll_by(&self, distance: Pixels) {
        self.list_state.scroll_by(distance);
    }

    pub(in crate::ui::shell) fn scroll_to(&self, item_ix: usize, offset_in_item: Pixels) {
        self.list_state.scroll_to(ListOffset {
            item_ix,
            offset_in_item,
        });
    }

    pub(in crate::ui::shell) fn scroll_to_reveal_message(&self, index: usize) {
        if index < self.messages.len() {
            self.list_state.scroll_to_reveal_item(index);
        }
    }

    pub(in crate::ui::shell) fn scroll_to_end(&self) {
        self.list_state.scroll_to_end();
    }

    pub(in crate::ui::shell) fn is_following_tail(&self) -> bool {
        self.list_state.is_following_tail()
    }

    /// Stops TextView notifications from repeatedly invalidating virtual-list measurements
    /// during a drag-selection. Call this as soon as the left-button selection gesture begins.
    pub(in crate::ui::shell) fn begin_selection_drag(&mut self) {
        self.selection_drag_active = true;
    }

    /// Ends the selection gate and remeasures every affected message once. Entity ids are kept
    /// instead of row indices because streaming/tool updates can insert or remove rows while the
    /// gesture is active.
    pub(in crate::ui::shell) fn finish_selection_drag(&mut self, cx: &mut Context<Self>) {
        if !self.selection_drag_active && self.deferred_remeasure.is_empty() {
            return;
        }

        self.selection_drag_active = false;
        let mut indices = std::mem::take(&mut self.deferred_remeasure)
            .into_iter()
            .filter_map(|message_id| self.message_indices.get(&message_id).copied())
            .collect::<Vec<_>>();
        if indices.is_empty() {
            return;
        }

        indices.sort_unstable();
        indices.dedup();

        let mut range_start = indices[0];
        let mut range_end = range_start + 1;
        for index in indices.into_iter().skip(1) {
            if index == range_end {
                range_end += 1;
            } else {
                self.list_state.remeasure_items(range_start..range_end);
                range_start = index;
                range_end = index + 1;
            }
        }
        self.list_state.remeasure_items(range_start..range_end);
        cx.notify();
    }

    pub(in crate::ui::shell) fn pause_tail_following(&self) -> bool {
        if !self.list_state.is_following_tail() {
            return false;
        }
        self.list_state.set_follow_mode(FollowMode::Normal);
        true
    }

    pub(in crate::ui::shell) fn restore_tail_follow_mode(&self) {
        restore_list_tail_follow_mode(&self.list_state);
    }

    fn subscribe_to_message(
        message: &Entity<SessionAgentMessageView>,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe(message, |this, message, event, cx| {
            let index = this.message_indices.get(&message.entity_id()).copied();
            let row_needs_refresh = index.is_some_and(|index| this.message_is_visible(index));
            let should_notify = match event {
                SessionAgentMessageViewEvent::ContentUpdated(update) => {
                    // Managed Markdown will notify again when the async incremental parse is
                    // ready. Waiting for that notification avoids rendering the same row once
                    // with stale parsed content and immediately again with the new document.
                    let waits_for_markdown =
                        matches!(update, SessionAgentContentUpdate::Appended { .. })
                            && message.read(cx).markdown_state().is_some();
                    if !waits_for_markdown && let Some(index) = index {
                        if this.selection_drag_active {
                            this.deferred_remeasure.insert(message.entity_id());
                            false
                        } else {
                            this.list_state.remeasure_items(index..index + 1);
                            row_needs_refresh
                        }
                    } else {
                        row_needs_refresh && !waits_for_markdown
                    }
                }
                SessionAgentMessageViewEvent::LayoutChanged => {
                    if let Some(index) = index {
                        if this.selection_drag_active {
                            this.deferred_remeasure.insert(message.entity_id());
                            false
                        } else {
                            this.list_state.remeasure_items(index..index + 1);
                            row_needs_refresh
                        }
                    } else {
                        false
                    }
                }
                SessionAgentMessageViewEvent::Refresh => row_needs_refresh,
            };
            if should_notify {
                cx.notify();
            }
        })
    }

    fn message_is_visible(&self, index: usize) -> bool {
        if self.list_state.is_following_tail() {
            return true;
        }

        matches!(self.list_state.item_is_above_viewport(index), Some(false))
            && matches!(self.list_state.item_is_below_viewport(index), Some(false))
    }

    fn rebuild_message_indices(&mut self) {
        self.message_indices.clear();
        self.message_indices.extend(
            self.messages
                .iter()
                .enumerate()
                .map(|(index, message)| (message.entity_id(), index)),
        );
    }
}

fn replace_generating_label_if_changed(current: &mut SharedString, next: SharedString) -> bool {
    if *current == next {
        return false;
    }

    *current = next;
    true
}

impl Render for SessionAgentConversationView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let conversation = cx.entity();
        list(self.list_state.clone(), move |index, _window, cx| {
            let (message, generating_view) = {
                let conversation = conversation.read(cx);
                let message = conversation.message(index);
                let generating_view = (message.is_none()
                    && conversation.generating
                    && index == conversation.messages.len())
                .then(|| conversation.generating_view.clone());
                (message, generating_view)
            };

            if let Some(message) = message {
                message.into_any_element()
            } else if let Some(generating_view) = generating_view {
                generating_view.into_any_element()
            } else {
                div().into_any_element()
            }
        })
        .size_full()
    }
}

fn estimate_tokens_from_chars(chars: usize) -> usize {
    chars.saturating_add(3) / 4
}

fn format_duration_ms(ms: u128) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_motion_is_consumed_without_restarting_the_same_key() {
        let mut motion = SessionAgentEnterMotionState::new(Some(7));
        assert_eq!(motion.active_key(), Some(7));
        assert_eq!(motion.active_key(), Some(7));

        motion = SessionAgentEnterMotionState::Playing {
            key: 7,
            started_at: Instant::now() - LIST_ENTER_DURATION - Duration::from_millis(1),
        };
        assert_eq!(motion.active_key(), None);
        motion.sync(Some(7));
        assert_eq!(motion.active_key(), None);

        motion.sync(Some(8));
        assert_eq!(motion.active_key(), Some(8));
    }

    #[test]
    fn only_never_started_enter_motion_is_retained_for_view_rebuild() {
        assert_eq!(
            SessionAgentEnterMotionState::Pending { key: 1 }.key_for_rebuild(),
            Some(1)
        );
        assert_eq!(
            SessionAgentEnterMotionState::Playing {
                key: 2,
                started_at: Instant::now(),
            }
            .key_for_rebuild(),
            None
        );
        assert_eq!(
            SessionAgentEnterMotionState::Consumed { key: 3 }.key_for_rebuild(),
            None
        );
        assert_eq!(SessionAgentEnterMotionState::Absent.key_for_rebuild(), None);
    }

    #[test]
    fn hidden_content_does_not_start_or_consume_enter_motion() {
        let mut motion = SessionAgentEnterMotionState::new(Some(9));
        let empty_assistant = SessionAgentMessage::assistant_raw("");

        assert!(!message_can_start_enter_motion(&empty_assistant));
        assert_eq!(
            motion.active_key_when(message_can_start_enter_motion(&empty_assistant)),
            None
        );
        assert_eq!(motion.key_for_rebuild(), Some(9));

        let visible_assistant = SessionAgentMessage::assistant_raw("hello");
        assert!(message_can_start_enter_motion(&visible_assistant));
        assert_eq!(
            motion.active_key_when(message_can_start_enter_motion(&visible_assistant)),
            Some(9)
        );
    }

    #[test]
    fn restoring_tail_mode_finishes_with_tail_following_enabled() {
        let list_state = ListState::new(3, ListAlignment::Top, px(0.0));
        list_state.set_follow_mode(FollowMode::Normal);
        list_state.scroll_to(ListOffset {
            item_ix: 1,
            offset_in_item: px(8.0),
        });

        restore_list_tail_follow_mode(&list_state);

        assert!(list_state.is_following_tail());
    }

    #[test]
    fn generating_label_only_reports_actual_locale_text_changes() {
        let mut current: SharedString = "Thinking".into();

        assert!(!replace_generating_label_if_changed(
            &mut current,
            "Thinking".into()
        ));
        assert_eq!(current, SharedString::from("Thinking"));

        assert!(replace_generating_label_if_changed(
            &mut current,
            "思考中".into()
        ));
        assert_eq!(current, SharedString::from("思考中"));
    }
}
