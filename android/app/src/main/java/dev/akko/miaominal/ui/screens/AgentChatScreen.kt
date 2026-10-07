package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.ui.uiString

import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.interaction.collectIsDraggedAsState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material.icons.outlined.Psychology
import androidx.compose.material.icons.outlined.AttachFile
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.material3.InputChip
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.material3.FilledIconButton
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.automirrored.outlined.Send
import androidx.compose.material.icons.outlined.AddComment
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.ModelTraining
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.StopCircle
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.ui.components.PageEmptyState
import dev.akko.miaominal.R
import kotlinx.coroutines.launch
import dev.akko.miaominal.data.agent.AgentModel
import dev.akko.miaominal.data.agent.AgentPhase
import dev.akko.miaominal.data.agent.AgentSessionState
import dev.akko.miaominal.data.agent.ContentBlock
import dev.akko.miaominal.data.agent.InteractionRequest
import dev.akko.miaominal.ui.agent.AgentComposerTag
import dev.akko.miaominal.ui.agent.AgentInteractionCard
import dev.akko.miaominal.ui.agent.AgentMessageListTag
import dev.akko.miaominal.ui.agent.AgentMessageView
import dev.akko.miaominal.ui.agent.AgentSendTag
import dev.akko.miaominal.ui.components.SessionTopBarCapsule
import dev.akko.miaominal.ui.agent.AgentReconnectTag

/**
 * Native chat for one AI CLI session.
 *
 * The screen is a pure function of [state]; every action is forwarded to the
 * repository, so switching pages or rebuilding the Activity never loses a
 * running turn.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AgentChatScreen(
    state: AgentSessionState,
    hostLabel: String,
    draft: String,
    onDraftChange: (String) -> Unit,
    onSend: () -> Unit,
    onStop: () -> Unit,
    onRespondOption: (String, String) -> Unit,
    onRespondAnswers: (String, Map<String, List<String>>) -> Unit,
    onSelectModel: (AgentModel) -> Unit,
    onCloseSession: () -> Unit,
    /** Opens the launch panel: new conversation or a history entry. */
    onSwitchSession: () -> Unit = {},
    onMenu: () -> Unit = {},
    onLeaveChat: () -> Unit = {},
    onReconnect: suspend () -> Result<Unit> = {
        Result.failure(IllegalStateException("Reconnect is unavailable"))
    },
    onSelectReasoning: (String) -> Unit = {},
    files: List<String> = emptyList(),
    onAttachFile: (() -> Unit)? = null,
    onRemoveFile: (String) -> Unit = {},
    modifier: Modifier = Modifier
) {
    val compactInput = dev.akko.miaominal.ui.components.LocalWorkspaceHeight.current < 500 &&
        WindowInsets.ime.getBottom(LocalDensity.current) > 0
    var errorDetails by remember { mutableStateOf(false) }
    val disconnected = state.connectionLost ||
        (state.session != null && state.phase != AgentPhase.Ready && state.phase != AgentPhase.Connecting)
    var reconnectAttempted by rememberSaveable(state.session?.remoteSessionId) { mutableStateOf(false) }
    var reconnectRequestError by rememberSaveable(state.session?.remoteSessionId) { mutableStateOf<String?>(null) }
    var reconnectRequestPending by remember { mutableStateOf(false) }
    LaunchedEffect(disconnected) {
        if (!disconnected) {
            reconnectAttempted = false
            reconnectRequestError = null
        }
    }
    val reconnecting = disconnected && (state.phase == AgentPhase.Connecting || reconnectRequestPending)
    val visibleError = (reconnectRequestError ?: state.error)?.takeIf { it.isNotBlank() && !reconnecting }
    val reconnectFailed = disconnected && reconnectAttempted && visibleError != null
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val dragged by listState.interactionSource.collectIsDraggedAsState()
    var followLatest by remember { mutableStateOf(true) }
    val scrollConnection = remember {
        object : NestedScrollConnection {
            override fun onPreScroll(available: Offset, source: NestedScrollSource): Offset {
                if (source == NestedScrollSource.UserInput && available.y > 0) followLatest = false
                return Offset.Zero
            }
        }
    }
    val interactionState = rememberSaveableStateHolder()
    var interactionOpen by rememberSaveable(state.pendingInteraction?.id) { mutableStateOf(true) }
    LaunchedEffect(dragged) {
        if (dragged) followLatest = false
    }
    LaunchedEffect(listState) {
        snapshotFlow { listState.isScrollInProgress to listState.canScrollForward }.collect { (scrolling, canScroll) ->
            if (!scrolling && !canScroll) followLatest = true
        }
    }
    val lastMessage = state.messages.lastOrNull()
    val lastContentLength = lastMessage?.blocks?.sumOf { block ->
        when (block) {
            is ContentBlock.Text -> block.text.length
            is ContentBlock.Reasoning -> block.text.length
            is ContentBlock.ToolCall -> block.output?.length ?: 0
            else -> 0
        }
    } ?: 0

    LaunchedEffect(state.messages.size, lastContentLength, state.pendingInteraction?.id, state.isBusy) {
        if (followLatest && !dragged && state.messages.isNotEmpty()) {
            val target = state.messages.size + if (state.notices.isNotEmpty()) 1 else 0
            runCatching { listState.animateScrollToItem(target) }
        }
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.surface)
            .statusBarsPadding()
            .navigationBarsPadding()
            .imePadding(),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        if (!compactInput) AgentChatTopBar(
            state = state,
            hostLabel = hostLabel,
            onCloseSession = onCloseSession,
            onMenu = onMenu,
            onSwitchSession = onSwitchSession,
            onLeaveChat = onLeaveChat
        )

        if (!compactInput && (disconnected || state.error != null)) {
            Surface(color = if (visibleError != null) MaterialTheme.colorScheme.errorContainer
                else MaterialTheme.colorScheme.surfaceContainerLow,
                shape = RoundedCornerShape(16.dp),
                modifier = Modifier.widthIn(max = 840.dp).fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
                Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        if (visibleError != null) Icon(Icons.Outlined.ErrorOutline, null,
                            tint = MaterialTheme.colorScheme.onErrorContainer,
                            modifier = Modifier.size(20.dp))
                        else if (reconnecting) CircularProgressIndicator(
                            modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                        Text(
                            when {
                                reconnecting -> stringResource(R.string.agent_reconnecting)
                                reconnectFailed -> stringResource(R.string.agent_reconnect_failed)
                                disconnected -> stringResource(R.string.agent_offline_draft)
                                else -> stringResource(R.string.agent_tool_failed)
                            },
                            style = MaterialTheme.typography.titleSmall,
                            color = if (visibleError != null) MaterialTheme.colorScheme.onErrorContainer
                            else MaterialTheme.colorScheme.onSurface,
                            modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite }
                        )
                    }
                    visibleError?.let { message ->
                        Text(message, maxLines = 2, overflow = TextOverflow.Ellipsis,
                            color = MaterialTheme.colorScheme.onErrorContainer,
                            style = MaterialTheme.typography.bodySmall,
                            modifier = Modifier.padding(top = 8.dp))
                    }
                    if (visibleError != null || disconnected) {
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End,
                            verticalAlignment = Alignment.CenterVertically) {
                            if (visibleError != null) TextButton(onClick = { errorDetails = true }) {
                                Text(stringResource(R.string.agent_launch_error_details))
                            }
                            if (disconnected && !reconnecting) TextButton(
                                onClick = {
                                    reconnectAttempted = true
                                    reconnectRequestError = null
                                    reconnectRequestPending = true
                                    errorDetails = false
                                    scope.launch {
                                        try {
                                            onReconnect().onFailure { failure ->
                                                reconnectRequestError = failure.message
                                                    ?.takeIf(String::isNotBlank) ?: failure.javaClass.simpleName
                                            }
                                        } catch (failure: Throwable) {
                                            if (failure is kotlinx.coroutines.CancellationException) throw failure
                                            reconnectRequestError = failure.message
                                                ?.takeIf(String::isNotBlank) ?: failure.javaClass.simpleName
                                        } finally {
                                            reconnectRequestPending = false
                                        }
                                    }
                                },
                                modifier = Modifier.testTag(AgentReconnectTag)) {
                                Text(stringResource(R.string.agent_reconnect))
                            }
                        }
                    }
                }
            }
        }

        Box(modifier = Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
            when {
                state.phase == AgentPhase.Connecting && state.messages.isEmpty() ->
                    Column(
                        modifier = Modifier.fillMaxSize(),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.Center
                    ) {
                        CircularProgressIndicator()
                        Spacer(Modifier.size(12.dp))
                        Text(uiString(R.string.ui_connecting), style = MaterialTheme.typography.bodyMedium)
                    }

                state.messages.isEmpty() && state.isBusy ->
                    Box(Modifier.fillMaxSize().padding(20.dp), contentAlignment = Alignment.TopStart) {
                        AgentActivityStatus(state.pendingInteraction != null)
                    }

                state.messages.isEmpty() -> PageEmptyState(
                    icon = Icons.Outlined.Psychology,
                    title = uiString(R.string.ui_start_a_new_conversation),
                    text = stringResource(R.string.agent_empty_hint),
                    modifier = Modifier.fillMaxSize()
                )

                else -> LazyColumn(
                    state = listState,
                    modifier = Modifier
                        .widthIn(max = 840.dp)
                        .fillMaxSize()
                        .nestedScroll(scrollConnection)
                        .testTag(AgentMessageListTag),
                    contentPadding = PaddingValues(horizontal = 20.dp, vertical = 20.dp),
                    verticalArrangement = Arrangement.spacedBy(24.dp)
                ) {
                    items(state.messages, key = { it.id }) { message ->
                        SelectionContainer { AgentMessageView(message) }
                    }
                    if (state.notices.isNotEmpty()) item(key = "agent-events") {
                        dev.akko.miaominal.ui.agent.AgentEventHistory(state.notices)
                    }
                    item(key = "agent-bottom") {
                        if (state.isBusy) AgentActivityStatus(state.pendingInteraction != null)
                        else Spacer(Modifier.height(1.dp))
                    }
                }
            }
            if (!followLatest && state.messages.isNotEmpty()) {
                Surface(
                    modifier = Modifier.align(Alignment.BottomEnd).padding(12.dp),
                    shape = MaterialTheme.shapes.extraLarge,
                    tonalElevation = 3.dp
                ) {
                    TextButton(onClick = {
                        followLatest = true
                        scope.launch { listState.animateScrollToItem(state.messages.size + if (state.notices.isNotEmpty()) 1 else 0) }
                    }) { Text(stringResource(R.string.agent_latest)) }
                }
            }
        }

        dev.akko.miaominal.ui.agent.AgentInputPanel(
            state = state, draft = draft, files = files,
            onDraftChange = onDraftChange, onSend = onSend, onStop = onStop,
            onAttachFile = onAttachFile, onRemoveFile = onRemoveFile,
            onSelectModel = onSelectModel, onSelectReasoning = onSelectReasoning,
            onInteraction = { interactionOpen = true },
            compact = compactInput
        )
    }
    if (errorDetails) androidx.compose.material3.AlertDialog(
        onDismissRequest = { errorDetails = false },
        title = { Text(stringResource(when {
            reconnectFailed -> R.string.agent_reconnect_failed
            disconnected -> R.string.agent_disconnected
            else -> R.string.agent_tool_failed
        })) },
        text = { SelectionContainer { Text(visibleError ?: stringResource(R.string.agent_offline_draft)) } },
        confirmButton = { TextButton(onClick = { errorDetails = false }) { Text(stringResource(R.string.agent_close_detail)) } },
        dismissButton = { visibleError?.let { dev.akko.miaominal.ui.agent.AgentCopyButton(it) } }
    )
    state.pendingInteraction?.let { interaction ->
        if (interactionOpen) {
            ModalBottomSheet(onDismissRequest = { interactionOpen = false }) {
                Column(Modifier.fillMaxWidth().heightIn(max = 600.dp)
                    .verticalScroll(rememberScrollState()).imePadding().padding(12.dp)) {
                    interactionState.SaveableStateProvider(interaction.id) {
                        AgentInteractionCard(
                            request = interaction,
                            submitting = state.interactionSubmitting,
                            failure = state.error,
                            onOption = { option -> onRespondOption(interaction.id, option) },
                            onAnswers = { answers -> onRespondAnswers(interaction.id, answers) }
                        )
                    }
                    if (state.isBusy) TextButton(onClick = onStop) {
                        Text(stringResource(R.string.agent_stop))
                    }
                }
            }
        }
    }
}

@Composable
private fun AgentActivityStatus(waiting: Boolean) {
    Row(
        modifier = Modifier.padding(vertical = 8.dp).semantics { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        CircularProgressIndicator(
            modifier = Modifier.size(12.dp), strokeWidth = 1.5.dp,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Text(stringResource(if (waiting) R.string.agent_waiting else R.string.agent_working),
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AgentChatTopBar(
    state: AgentSessionState,
    hostLabel: String,
    onCloseSession: () -> Unit,
    onMenu: () -> Unit,
    onSwitchSession: () -> Unit,
    onLeaveChat: () -> Unit
) {
    var menuOpen by remember { mutableStateOf(false) }
    SessionTopBarCapsule {
            IconButton(onClick = onMenu) {
                Icon(
                    Icons.Outlined.Menu,
                    contentDescription = stringResource(R.string.links_sidebar_toggle),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp)
            ) {
                Text(
                    text = listOfNotNull(state.session?.cli?.displayName, hostLabel.takeIf { it.isNotBlank() }).joinToString(" · ")
                        .ifBlank { uiString(R.string.ui_ai_session) },
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
                Text(
                    text = state.session?.cwd.orEmpty(),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
            }
            Box {
                IconButton(onClick = { menuOpen = true }) {
                    Icon(Icons.Outlined.MoreVert, contentDescription = uiString(R.string.ui_session_actions))
                }
                DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                    DropdownMenuItem(text = { Text(stringResource(R.string.agent_leave_chat)) },
                        onClick = { menuOpen = false; onLeaveChat() })
                    DropdownMenuItem(
                        text = { Text(uiString(R.string.ui_new_or_switch_session)) },
                        leadingIcon = { Icon(Icons.Outlined.AddComment, contentDescription = null) },
                        enabled = state.phase != AgentPhase.Connecting,
                        onClick = { menuOpen = false; onSwitchSession() }
                    )
                    DropdownMenuItem(
                        text = { Text(uiString(R.string.ui_close_session)) },
                        leadingIcon = { Icon(Icons.Outlined.Close, contentDescription = null) },
                        onClick = { menuOpen = false; onCloseSession() }
                    )
                }
            }
    }
}
