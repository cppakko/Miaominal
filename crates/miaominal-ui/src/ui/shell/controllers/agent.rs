use super::AppCommand;
use crate::ui::i18n;
use crate::ui::shell::session_agent_view::SessionAgentConversationView;
use crate::ui::shell::{
    AppIcon, LocalVaultStatus, SESSION_MONITOR_PANEL_WIDTH, SelectOption, SessionQueryPort,
    SessionTerminalPort, TerminalSearchAnimation, WorkspaceSidePanelTransition, new_input_state,
    set_input_placeholder,
};
use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, IntoElement,
    Pixels, Point, Render, Subscription, WeakEntity, Window, div, px,
};
use gpui_component::{
    IndexPath, VirtualListScrollHandle,
    input::{InputEvent, InputState},
    select::{SelectEvent, SelectState},
};
use miaominal_agent::AgentMode;
use miaominal_secrets::SecretStore;
use miaominal_services::{AgentService, ChatService};
use miaominal_storage::chat_store::ChatSessionRecord;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use std::cell::{Ref, RefCell, RefMut};
use tokio::runtime::Handle as TokioHandle;

mod attachments;
mod conversation;
mod history;
mod prompt;
mod recovery;
mod runtime;
mod search;
mod session_ports;
mod stream;
mod tasks;
mod tool_interaction;

#[cfg(test)]
pub(in crate::ui::shell) use history::{
    chat_record_from_session_agent_message, restored_tool_status_and_note,
    session_agent_message_from_record, tool_status_as_str,
};
pub(in crate::ui::shell) use prompt::PromptHistoryDirection;
pub(in crate::ui::shell) use prompt::{
    AgentPromptDraft, AgentPromptDraftOutcome, AgentPromptRequestPreparation,
};
use runtime::AgentRuntimeStore;
pub(in crate::ui::shell) use runtime::{
    AgentExecMode, ChatPanelView, SessionAgentExecutionContext, SessionAgentMessage,
    SessionAgentMessageMotion, SessionAgentMessageRole, SessionAgentState, SessionAgentToolCall,
    SessionAgentToolStatus, split_message_into_blocks, trailing_at_mention_query,
};
pub(in crate::ui::shell) use stream::{
    AgentStreamFollowUp, SessionAgentBackgroundNotificationKind,
};
pub(in crate::ui::shell) use tasks::{
    AgentApprovedToolTask, AgentStreamTask, AgentStreamTaskRequest,
};
pub(in crate::ui::shell) use tool_interaction::{
    AgentContinuationPreparation, AgentToolApprovalCommit, AgentToolContinuation,
    AgentToolExecutionCommit,
};

pub(in crate::ui::shell) struct WorkspaceAgentForms {
    pub(in crate::ui::shell) prompt_input: Entity<InputState>,
    pub(in crate::ui::shell) ask_user_input: Entity<InputState>,
    pub(in crate::ui::shell) title_input: Entity<InputState>,
    pub(in crate::ui::shell) rename_title_input: Entity<InputState>,
    pub(in crate::ui::shell) editing_title: bool,
    pub(in crate::ui::shell) agent_mode_select: Entity<SelectState<Vec<SelectOption<AgentMode>>>>,
    pub(in crate::ui::shell) chat_search: ChatSearchForms,
}

pub(in crate::ui::shell) struct ChatSearchForms {
    pub(in crate::ui::shell) session_filter_input: Entity<InputState>,
    pub(in crate::ui::shell) session_filter_open: bool,
    pub(in crate::ui::shell) session_filter_visible: bool,
    pub(in crate::ui::shell) session_filter_visibility: f32,
    pub(in crate::ui::shell) session_filter_animation: Option<TerminalSearchAnimation>,
    pub(in crate::ui::shell) conversation_search_input: Entity<InputState>,
    pub(in crate::ui::shell) conversation_search_open: bool,
    pub(in crate::ui::shell) conversation_search_visible: bool,
    pub(in crate::ui::shell) conversation_search_visibility: f32,
    pub(in crate::ui::shell) conversation_search_animation: Option<TerminalSearchAnimation>,
    pub(in crate::ui::shell) match_count: usize,
    pub(in crate::ui::shell) current_match: Option<usize>,
    pub(in crate::ui::shell) status: Option<String>,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingChatSessionDeleteState {
    pub(in crate::ui::shell) session_id: String,
    pub(in crate::ui::shell) title: String,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingChatSessionRenameState {
    pub(in crate::ui::shell) session_id: String,
    pub(in crate::ui::shell) current_title: String,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SessionAgentPanelDragState {
    pub(in crate::ui::shell) initial_pointer: f32,
    pub(in crate::ui::shell) initial_width: f32,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SessionAgentAutoScrollState {
    pub(in crate::ui::shell) anchor_y: f32,
    pub(in crate::ui::shell) pointer_y: f32,
    pub(in crate::ui::shell) generation: u64,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct SessionAgentTargetCandidate {
    pub(in crate::ui::shell) name: String,
    pub(in crate::ui::shell) detail: String,
    pub(in crate::ui::shell) resolved: bool,
}

pub(in crate::ui::shell) struct AgentFinishStreamOutcome {
    pub(in crate::ui::shell) title_seed: Option<(String, String)>,
}

struct AgentPanelLayoutState {
    open: bool,
    visible: bool,
    transition: Option<WorkspaceSidePanelTransition>,
    width: f32,
    drag: Option<SessionAgentPanelDragState>,
    auto_scroll: Option<SessionAgentAutoScrollState>,
    auto_scroll_generation: u64,
    text_drag_origin: Option<Point<Pixels>>,
    text_drag_conversation: Option<Entity<SessionAgentConversationView>>,
    text_drag_paused_tail: bool,
}

struct AgentConversationViewObserver;

impl AgentConversationViewObserver {
    fn observe(
        &mut self,
        view: &Entity<SessionAgentConversationView>,
        controller: WeakEntity<AgentController>,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.observe(view, move |_observer, observed_view, cx| {
            let controller = controller.clone();
            let observed_view = observed_view.clone();
            cx.defer(move |cx| {
                controller
                    .update(cx, |controller, cx| {
                        if controller
                            .runtime
                            .borrow()
                            .foreground
                            .conversation_view
                            .as_ref()
                            .is_some_and(|active_view| {
                                active_view.entity_id() == observed_view.entity_id()
                            })
                        {
                            cx.notify();
                        }
                    })
                    .ok();
            });
        })
    }
}

pub(in crate::ui::shell) struct AgentControllerArgs {
    pub(in crate::ui::shell) task_runtime: TokioHandle,
    pub(in crate::ui::shell) agent_service: AgentService,
    pub(in crate::ui::shell) secrets: SecretStore,
    pub(in crate::ui::shell) known_hosts: KnownHostsStore,
    pub(in crate::ui::shell) chat_service: Option<ChatService>,
    pub(in crate::ui::shell) chat_sessions: Vec<ChatSessionRecord>,
    pub(in crate::ui::shell) local_vault_status: LocalVaultStatus,
}

pub(in crate::ui::shell) struct AgentController {
    task_runtime: TokioHandle,
    agent_service: AgentService,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    local_vault_status: LocalVaultStatus,
    session_query: SessionQueryPort,
    session_terminal: SessionTerminalPort,
    forms: WorkspaceAgentForms,
    focus: FocusHandle,
    chat_service: Option<ChatService>,
    chat_sessions: Vec<ChatSessionRecord>,
    runtime: RefCell<AgentRuntimeStore>,
    conversation_view_observer: Entity<AgentConversationViewObserver>,
    history_scroll_handle: VirtualListScrollHandle,
    panel_layout: RefCell<AgentPanelLayoutState>,
    pending_chat_session_delete: Option<PendingChatSessionDeleteState>,
    pending_chat_session_rename: Option<PendingChatSessionRenameState>,
    _subscriptions: Vec<Subscription>,
}

impl AgentController {
    fn build_forms(window: &mut Window, cx: &mut Context<Self>) -> WorkspaceAgentForms {
        let prompt_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(3, 8)
                .submit_on_enter(true)
                .context_menu(false)
                .placeholder("")
        });
        set_input_placeholder(
            &prompt_input,
            i18n::string("workspace.panel.agent.placeholder"),
            window,
            cx,
        );
        let ask_user_input = cx.new(|cx| {
            InputState::new(window, cx)
                .submit_on_enter(true)
                .placeholder("")
        });
        set_input_placeholder(
            &ask_user_input,
            i18n::string("workspace.panel.agent.tool_placeholders.custom_answer"),
            window,
            cx,
        );
        let agent_mode_options = vec![
            SelectOption::new_with_icon(
                AgentMode::Ask,
                i18n::string("agent.mode.ask"),
                AppIcon::Eye,
            ),
            SelectOption::new_with_icon(
                AgentMode::Execute,
                i18n::string("agent.mode.execute"),
                AppIcon::Play,
            ),
            SelectOption::new_with_icon(
                AgentMode::NonBlocking,
                i18n::string("agent.mode.non_blocking"),
                AppIcon::Sliders,
            ),
            SelectOption::new_with_icon(
                AgentMode::FullAuto,
                i18n::string("agent.mode.full_auto"),
                AppIcon::Sparkles,
            ),
        ];

        WorkspaceAgentForms {
            prompt_input,
            ask_user_input,
            title_input: new_input_state(
                i18n::string("workspace.panel.agent.sidebar_title"),
                "",
                false,
                window,
                cx,
            ),
            rename_title_input: new_input_state(
                i18n::string("dialogs.chat_rename.title_label"),
                "",
                false,
                window,
                cx,
            ),
            editing_title: false,
            agent_mode_select: cx.new(|cx| {
                SelectState::new(
                    agent_mode_options,
                    Some(IndexPath::default().row(1)),
                    window,
                    cx,
                )
            }),
            chat_search: ChatSearchForms {
                session_filter_input: new_input_state(
                    i18n::string("placeholders.workspace.search_sessions"),
                    "",
                    false,
                    window,
                    cx,
                ),
                session_filter_open: false,
                session_filter_visible: false,
                session_filter_visibility: 0.0,
                session_filter_animation: None,
                conversation_search_input: new_input_state(
                    i18n::string("placeholders.workspace.search_messages"),
                    "",
                    false,
                    window,
                    cx,
                ),
                conversation_search_open: false,
                conversation_search_visible: false,
                conversation_search_visibility: 0.0,
                conversation_search_animation: None,
                match_count: 0,
                current_match: None,
                status: None,
            },
        }
    }

    pub(in crate::ui::shell) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, key) in [
            (
                &self.forms.prompt_input,
                "workspace.panel.agent.placeholder",
            ),
            (
                &self.forms.ask_user_input,
                "workspace.panel.agent.tool_placeholders.custom_answer",
            ),
            (
                &self.forms.chat_search.session_filter_input,
                "placeholders.workspace.search_sessions",
            ),
            (
                &self.forms.chat_search.conversation_search_input,
                "placeholders.workspace.search_messages",
            ),
        ] {
            set_input_placeholder(input, i18n::string(key), window, cx);
        }
    }

    pub(in crate::ui::shell) fn new(
        args: AgentControllerArgs,
        session_query: SessionQueryPort,
        session_terminal: SessionTerminalPort,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let forms = Self::build_forms(window, cx);
        let conversation_view_observer = cx.new(|_| AgentConversationViewObserver);
        let prompt_input = forms.prompt_input.clone();
        let ask_user_input = forms.ask_user_input.clone();
        let title_input = forms.title_input.clone();
        let agent_mode_select = forms.agent_mode_select.clone();
        let session_filter_input = forms.chat_search.session_filter_input.clone();
        let conversation_search_input = forms.chat_search.conversation_search_input.clone();
        let subscriptions = vec![
            cx.subscribe(
                &prompt_input,
                |controller, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        controller.prompt_input_changed(&value);
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &ask_user_input,
                |_controller, _input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &title_input,
                |controller, input, event: &InputEvent, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        let value = input.read(cx).value().trim().to_string();
                        controller.update_session_agent_title(
                            if value.is_empty() { None } else { Some(value) },
                            cx,
                        );
                        controller.forms.editing_title = false;
                        cx.notify();
                    }
                    InputEvent::Blur => {
                        controller.forms.editing_title = false;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe(
                &agent_mode_select,
                |controller,
                 select,
                 _event: &SelectEvent<
                    Vec<crate::ui::shell::SelectOption<miaominal_agent::AgentMode>>,
                >,
                 cx| {
                    if let Some(mode) = select.read(cx).selected_value().cloned() {
                        controller.runtime.get_mut().foreground.agent_mode = mode;
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &session_filter_input,
                |controller, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        controller.runtime.get_mut().foreground.search_query =
                            if value.is_empty() { None } else { Some(value) };
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &conversation_search_input,
                |controller, input, event: &InputEvent, cx| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        controller.update_conversation_search(value, cx);
                    }
                    InputEvent::PressEnter {
                        secondary,
                        shift: _,
                    } => {
                        if *secondary {
                            controller.navigate_conversation_search_prev(cx);
                        } else {
                            controller.navigate_conversation_search_next(cx);
                        }
                    }
                    _ => {}
                },
            ),
        ];
        Self {
            task_runtime: args.task_runtime,
            agent_service: args.agent_service,
            secrets: args.secrets,
            known_hosts: args.known_hosts,
            local_vault_status: args.local_vault_status,
            session_query,
            session_terminal,
            forms,
            focus: cx.focus_handle(),
            chat_service: args.chat_service,
            chat_sessions: args.chat_sessions,
            runtime: RefCell::new(AgentRuntimeStore::default()),
            conversation_view_observer,
            history_scroll_handle: VirtualListScrollHandle::new(),
            panel_layout: RefCell::new(AgentPanelLayoutState {
                open: false,
                visible: false,
                transition: None,
                width: SESSION_MONITOR_PANEL_WIDTH,
                drag: None,
                auto_scroll: None,
                auto_scroll_generation: 0,
                text_drag_origin: None,
                text_drag_conversation: None,
                text_drag_paused_tail: false,
            }),
            pending_chat_session_delete: None,
            pending_chat_session_rename: None,
            _subscriptions: subscriptions,
        }
    }

    pub(in crate::ui::shell) fn copy_text(
        &mut self,
        label: String,
        text: String,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "workspace.panel.agent.messages.copy_success",
            &[("label", &label)],
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn history_scroll_handle(&self) -> VirtualListScrollHandle {
        self.history_scroll_handle.clone()
    }

    pub(in crate::ui::shell) fn panel_width(&self) -> f32 {
        self.panel_layout.borrow().width
    }

    pub(in crate::ui::shell) fn panel_open(&self) -> bool {
        self.panel_layout.borrow().open
    }

    pub(in crate::ui::shell) fn set_panel_open(&self, open: bool) {
        self.panel_layout.borrow_mut().open = open;
    }

    pub(in crate::ui::shell) fn toggle_panel(&self) {
        let mut layout = self.panel_layout.borrow_mut();
        layout.open = !layout.open;
    }

    pub(in crate::ui::shell) fn panel_transition_state(
        &self,
    ) -> (bool, Option<WorkspaceSidePanelTransition>) {
        let layout = self.panel_layout.borrow();
        (layout.visible, layout.transition)
    }

    pub(in crate::ui::shell) fn set_panel_transition_state(
        &self,
        visible: bool,
        transition: Option<WorkspaceSidePanelTransition>,
    ) {
        let mut layout = self.panel_layout.borrow_mut();
        layout.visible = visible;
        layout.transition = transition;
    }

    pub(in crate::ui::shell) fn set_panel_width(&self, width: f32) {
        self.panel_layout.borrow_mut().width = width;
    }

    pub(in crate::ui::shell) fn panel_drag(&self) -> Option<SessionAgentPanelDragState> {
        self.panel_layout.borrow().drag.clone()
    }

    pub(in crate::ui::shell) fn set_panel_drag(&self, drag: Option<SessionAgentPanelDragState>) {
        self.panel_layout.borrow_mut().drag = drag;
    }

    pub(in crate::ui::shell) fn take_panel_drag(&self) -> Option<SessionAgentPanelDragState> {
        self.panel_layout.borrow_mut().drag.take()
    }

    pub(in crate::ui::shell) fn start_auto_scroll(&self, pointer_y: f32) -> u64 {
        let mut layout = self.panel_layout.borrow_mut();
        layout.auto_scroll_generation = layout.auto_scroll_generation.wrapping_add(1);
        let generation = layout.auto_scroll_generation;
        layout.auto_scroll = Some(SessionAgentAutoScrollState {
            anchor_y: pointer_y,
            pointer_y,
            generation,
        });
        generation
    }

    pub(in crate::ui::shell) fn update_auto_scroll_pointer(&self, pointer_y: f32) -> bool {
        let mut layout = self.panel_layout.borrow_mut();
        let Some(auto_scroll) = layout.auto_scroll.as_mut() else {
            return false;
        };
        auto_scroll.pointer_y = pointer_y;
        true
    }

    pub(in crate::ui::shell) fn stop_auto_scroll(&self) -> bool {
        let mut layout = self.panel_layout.borrow_mut();
        if layout.auto_scroll.take().is_none() {
            return false;
        }
        layout.auto_scroll_generation = layout.auto_scroll_generation.wrapping_add(1);
        true
    }

    pub(in crate::ui::shell) fn auto_scroll(&self) -> Option<SessionAgentAutoScrollState> {
        self.panel_layout.borrow().auto_scroll.clone()
    }

    pub(in crate::ui::shell) fn begin_text_drag(
        &self,
        origin: Point<Pixels>,
        conversation: Option<Entity<SessionAgentConversationView>>,
    ) {
        let mut layout = self.panel_layout.borrow_mut();
        layout.text_drag_origin = Some(origin);
        layout.text_drag_conversation = conversation;
        layout.text_drag_paused_tail = false;
    }

    pub(in crate::ui::shell) fn text_drag_active(&self) -> bool {
        let layout = self.panel_layout.borrow();
        layout.text_drag_origin.is_some() || layout.text_drag_conversation.is_some()
    }

    pub(in crate::ui::shell) fn text_drag_origin(&self) -> Option<Point<Pixels>> {
        self.panel_layout.borrow().text_drag_origin
    }

    pub(in crate::ui::shell) fn text_drag_conversation(
        &self,
    ) -> Option<Entity<SessionAgentConversationView>> {
        self.panel_layout.borrow().text_drag_conversation.clone()
    }

    pub(in crate::ui::shell) fn text_drag_paused_tail(&self) -> bool {
        self.panel_layout.borrow().text_drag_paused_tail
    }

    pub(in crate::ui::shell) fn set_text_drag_paused_tail(&self, paused: bool) {
        self.panel_layout.borrow_mut().text_drag_paused_tail = paused;
    }

    pub(in crate::ui::shell) fn take_text_drag(
        &self,
    ) -> (Option<Entity<SessionAgentConversationView>>, bool) {
        let mut layout = self.panel_layout.borrow_mut();
        layout.text_drag_origin = None;
        let conversation = layout.text_drag_conversation.take();
        let paused_tail = std::mem::take(&mut layout.text_drag_paused_tail);
        (conversation, paused_tail)
    }

    pub(in crate::ui::shell) fn finish_text_drag(&mut self, cx: &mut Context<Self>) {
        let (conversation, paused_tail) = self.take_text_drag();
        let Some(conversation) = conversation else {
            return;
        };
        conversation.update(cx, |view, cx| view.finish_selection_drag(cx));
        if paused_tail {
            conversation.read(cx).restore_tail_follow_mode();
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn agent_service(&self) -> AgentService {
        self.agent_service.clone()
    }

    pub(in crate::ui::shell) fn secrets(&self) -> SecretStore {
        self.secrets.clone()
    }

    pub(in crate::ui::shell) fn known_hosts(&self) -> KnownHostsStore {
        self.known_hosts.clone()
    }

    pub(in crate::ui::shell) fn local_vault_status(&self) -> LocalVaultStatus {
        self.local_vault_status
    }

    pub(in crate::ui::shell) fn credentials_changed(
        &mut self,
        secrets: SecretStore,
        local_vault_status: LocalVaultStatus,
        cx: &mut Context<Self>,
    ) {
        self.secrets = secrets.clone();
        self.local_vault_status = local_vault_status;
        self.agent_service =
            AgentService::new(self.task_runtime.clone(), secrets, self.known_hosts.clone());
        cx.notify();
    }

    pub(in crate::ui::shell) fn rename_title_input(&self) -> Entity<InputState> {
        self.forms.rename_title_input.clone()
    }

    pub(in crate::ui::shell) fn focus(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub(in crate::ui::shell) fn prompt_input(&self) -> Entity<InputState> {
        self.forms.prompt_input.clone()
    }

    pub(in crate::ui::shell) fn ask_user_input(&self) -> Entity<InputState> {
        self.forms.ask_user_input.clone()
    }

    pub(in crate::ui::shell) fn title_input(&self) -> Entity<InputState> {
        self.forms.title_input.clone()
    }

    pub(in crate::ui::shell) fn agent_mode_select(
        &self,
    ) -> Entity<
        gpui_component::select::SelectState<
            Vec<crate::ui::shell::SelectOption<miaominal_agent::AgentMode>>,
        >,
    > {
        self.forms.agent_mode_select.clone()
    }

    pub(in crate::ui::shell) fn editing_title(&self) -> bool {
        self.forms.editing_title
    }

    pub(in crate::ui::shell) fn set_editing_title(
        &mut self,
        editing: bool,
        cx: &mut Context<Self>,
    ) {
        self.forms.editing_title = editing;
        cx.notify();
    }

    pub(in crate::ui::shell) fn chat_sessions(&self) -> &[ChatSessionRecord] {
        &self.chat_sessions
    }

    pub(in crate::ui::shell) fn chat_history_available(&self) -> bool {
        self.chat_service.is_some()
    }

    pub(in crate::ui::shell) fn update_session_agent_title(
        &mut self,
        title: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let session_id = {
            let state = &mut self.runtime.get_mut().foreground;
            state.title = title.clone();
            state.session_id.clone()
        };
        let Some(session_id) = session_id else {
            cx.notify();
            return;
        };
        if !self.chat_history_available() {
            cx.notify();
            return;
        }
        if let Err(error) =
            self.rename_chat_session_record(&session_id, title.as_deref().unwrap_or(""), cx)
        {
            log::warn!("failed to update chat title: {error:?}");
        }
    }

    pub(in crate::ui::shell) fn session_agent(&self) -> Ref<'_, SessionAgentState> {
        Ref::map(self.runtime.borrow(), |runtime| &runtime.foreground)
    }

    pub(in crate::ui::shell) fn session_agent_mut(&self) -> RefMut<'_, SessionAgentState> {
        RefMut::map(self.runtime.borrow_mut(), |runtime| &mut runtime.foreground)
    }

    pub(in crate::ui::shell) fn foreground_session_id(&self) -> Option<String> {
        self.runtime.borrow().foreground.session_id.clone()
    }

    pub(in crate::ui::shell) fn session_is_foreground(&self, session_id: &str) -> bool {
        self.runtime.borrow().session_is_foreground(session_id)
    }

    pub(in crate::ui::shell) fn session_exists(&self, session_id: &str) -> bool {
        self.runtime.borrow().session(session_id).is_some()
    }

    pub(in crate::ui::shell) fn session_panel_view(
        &self,
        session_id: &str,
    ) -> Option<ChatPanelView> {
        self.runtime
            .borrow()
            .session(session_id)
            .map(|state| state.panel_view)
    }

    pub(in crate::ui::shell) fn session_has_active_tool_call(&self, session_id: &str) -> bool {
        self.runtime
            .borrow()
            .session(session_id)
            .is_some_and(SessionAgentState::has_active_tool_call)
    }

    pub(in crate::ui::shell) fn session_chat_label(&self, session_id: &str) -> Option<String> {
        let state = self.runtime.borrow();
        let state = state.session(session_id)?;
        if let Some(title) = state
            .title
            .as_ref()
            .map(|title| title.trim())
            .filter(|title| !title.is_empty())
        {
            return Some(crate::ui::shell::truncate_with_ellipsis(title, 48));
        }

        let fallback_title = i18n::string("workspace.panel.agent.sidebar_title");
        Some(
            state
                .messages
                .iter()
                .find(|message| message.role == SessionAgentMessageRole::User)
                .map(|message| {
                    crate::ui::shell::truncate_with_ellipsis(
                        message
                            .content
                            .lines()
                            .next()
                            .unwrap_or(fallback_title.as_str())
                            .trim(),
                        48,
                    )
                })
                .filter(|title| !title.is_empty())
                .unwrap_or(fallback_title),
        )
    }

    pub(in crate::ui::shell) fn conversation_search_scroll_target(&self) -> Option<(usize, usize)> {
        let state = self.session_agent();
        (state.panel_view == ChatPanelView::Conversation)
            .then_some(state.search_scroll_target)
            .flatten()
    }

    pub(in crate::ui::shell) fn dismiss_at_mention(&mut self, cx: &mut Context<Self>) -> bool {
        let state = &mut self.runtime.get_mut().foreground;
        if state.at_mention_query.take().is_none() {
            return false;
        }
        state.at_mention_anchor = 0;
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn toggle_execution_mode(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.runtime.get_mut().foreground;
        state.exec_mode = state.exec_mode.toggle();
        cx.notify();
    }

    pub(in crate::ui::shell) fn toggle_thinking_expanded(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<SessionAgentMessage> {
        let state = &mut self.runtime.get_mut().foreground;
        state.toggle_thinking_expanded(index);
        let snapshot = state.messages.get(index).cloned();
        cx.notify();
        snapshot
    }

    pub(in crate::ui::shell) fn toggle_tool_call_expanded(
        &mut self,
        tool_id: &str,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<SessionAgentMessage> {
        let state = &mut self.runtime.get_mut().foreground;
        state.toggle_tool_call_expanded(tool_id);
        let snapshot = state.messages.get(index).cloned();
        cx.notify();
        snapshot
    }

    pub(in crate::ui::shell) fn ensure_conversation_view(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Entity<SessionAgentConversationView> {
        let observer = self.conversation_view_observer.clone();
        let controller = cx.weak_entity();
        let state = &mut self.runtime.get_mut().foreground;
        let search_layout_active = state
            .search_query
            .as_ref()
            .is_some_and(|query| !query.trim().is_empty());
        let view = if let Some(view) = state.conversation_view.as_ref() {
            view.clone()
        } else {
            let messages = state.messages.clone();
            let generating = state.has_pending_task();
            let viewport = state.conversation_viewport.take();
            let view = cx.new(move |cx| {
                SessionAgentConversationView::from_messages(messages, generating, cx)
            });
            if let Some(viewport) = viewport
                && !viewport.following_tail
            {
                let offset = viewport.offset_for_search_layout(search_layout_active);
                view.read(cx)
                    .scroll_to(offset.item_ix, offset.offset_in_item);
            }
            state.conversation_view = Some(view.clone());
            view
        };

        if state.conversation_view_observation.is_none() {
            state.conversation_view_observation =
                Some(observer.update(cx, |observer, cx| observer.observe(&view, controller, cx)));
        }
        if let Some((message_index, _)) = state.search_scroll_target {
            view.read(cx).scroll_to(message_index, gpui::px(0.0));
        }
        let generating_label = i18n::string("workspace.panel.agent.thinking");
        view.update(cx, |view, cx| {
            view.set_generating_label(generating_label, cx);
        });
        view
    }

    pub(in crate::ui::shell) fn ensure_panel_conversation_view(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Entity<SessionAgentConversationView> {
        let view = self.ensure_conversation_view(cx);
        let search_scroll_target = self.conversation_search_scroll_target();
        if self.panel_open()
            && self.session_query.has_active_terminal_session()
            && self.text_drag_conversation().is_none()
            && let Some((message_index, _)) = search_scroll_target
        {
            view.read(cx).scroll_to(message_index, px(0.0));
        }
        view
    }

    pub(in crate::ui::shell) fn push_conversation_message_view(
        &mut self,
        message: SessionAgentMessage,
        cx: &mut Context<Self>,
    ) {
        self.runtime
            .get_mut()
            .foreground
            .push_conversation_message_view(message, cx);
    }

    pub(in crate::ui::shell) fn sync_conversation_message_view(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        self.runtime
            .get_mut()
            .foreground
            .sync_conversation_message_view(index, cx);
    }

    pub(in crate::ui::shell) fn append_conversation_message_view_delta(
        &mut self,
        index: usize,
        delta: &str,
        cx: &mut Context<Self>,
    ) {
        self.runtime
            .get_mut()
            .foreground
            .append_conversation_message_view_delta(index, delta, cx);
    }

    pub(in crate::ui::shell) fn clear_conversation_view(&mut self, cx: &mut Context<Self>) {
        self.runtime
            .get_mut()
            .foreground
            .clear_conversation_view(cx);
    }

    pub(in crate::ui::shell) fn release_conversation_view(&mut self, cx: &mut Context<Self>) {
        self.runtime
            .get_mut()
            .foreground
            .release_conversation_view(cx);
    }

    fn sync_conversation_generating_view(&mut self, cx: &mut Context<Self>) {
        self.runtime
            .get_mut()
            .foreground
            .sync_conversation_generating_view(cx);
    }

    pub(in crate::ui::shell) fn background_session_is_busy(&self, session_id: &str) -> bool {
        self.runtime.borrow().background_session_is_busy(session_id)
    }

    pub(in crate::ui::shell) fn background_session_needs_approval(&self, session_id: &str) -> bool {
        self.runtime
            .borrow()
            .background_session_needs_approval(session_id)
    }
}

impl EventEmitter<AppCommand> for AgentController {}

impl Render for AgentController {
    fn render(&mut self, _window: &mut gpui::Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
