package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.ui.components.ConnectionErrorActions
import dev.akko.miaominal.ui.theme.LocalUiPreferences
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.foundation.text.selection.SelectionContainer

import android.os.SystemClock
import dev.akko.miaominal.ui.components.WorkspaceBackHandler as BackHandler
import androidx.annotation.StringRes
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.LocalIndication
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.indication
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import dev.akko.miaominal.R
import dev.akko.miaominal.data.SessionPhase
import dev.akko.miaominal.data.TerminalSessionHandle
import dev.akko.miaominal.ui.components.OpenedLinkKind
import dev.akko.miaominal.ui.components.OpenedLinkStatus
import dev.akko.miaominal.ui.components.OpenedLinkUi
import dev.akko.miaominal.ui.components.OpenedLinksDrawer
import dev.akko.miaominal.ui.components.SessionDrawer
import dev.akko.miaominal.ui.components.SessionIdentity
import dev.akko.miaominal.ui.components.SessionTopBarHeight
import dev.akko.miaominal.ui.components.SessionTopBarCapsule
import dev.akko.miaominal.ui.components.TerminalSurfaceView
import dev.akko.miaominal.ui.components.TerminalTextExplosionDialog
import dev.akko.miaominal.ui.components.TerminalTextExplosionViewModel
import dev.akko.miaominal.ui.model.HostProfileUi
import dev.akko.miaominal.ui.model.TerminalResizeQueue
import dev.akko.miaominal.ui.model.pumpTerminalFrames
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.withContext


private data class TerminalSnippet(@param:StringRes val labelRes: Int, val command: String)

/** One entry in the multi-session drawer. */
data class TerminalSessionTab(
    val id: Long,
    val title: String,
    val subtitle: String,
    val active: Boolean,
    val waiting: Boolean
)

private val TerminalSnippets = listOf(
    TerminalSnippet(R.string.terminal_snippet_list_files, "ls -la"),
    TerminalSnippet(R.string.terminal_snippet_uptime, "uptime"),
    TerminalSnippet(R.string.terminal_snippet_git_status, "git status"),
    TerminalSnippet(R.string.terminal_snippet_docker_ps, "docker ps")
)
/** Keys that keep sending while held down. */
private val RepeatableKeys = setOf("LEFT", "UP", "DOWN", "RIGHT", "PAGEUP", "PAGEDOWN")

/** Hold delay before a repeatable key starts repeating. */
private const val KeyRepeatStartDelayMs = 350L

/** Interval between repeats while a key stays held. */
private const val KeyRepeatIntervalMs = 80L

/** Gesture poll interval; also the resolution of the hold timer. */
private const val KeyRepeatPollMs = 50L

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TerminalScreen(
    profile: HostProfileUi,
    session: TerminalSessionHandle?,
    sessionTabs: List<TerminalSessionTab>,
    onSwitchSession: (Long) -> Unit,
    onCloseSession: (Long) -> Unit,
    onDismiss: () -> Unit,
    onOpenSftp: () -> Unit,
    onOpenAgent: (() -> Unit)? = null,
    onReconnect: () -> Unit = {},
    onEditProfile: () -> Unit = {},
    sftpLinks: List<OpenedLinkUi> = emptyList(),
    agentLinks: List<OpenedLinkUi> = emptyList(),
    onSelectAgent: () -> Unit = {},
    onCloseAgent: () -> Unit = {},
    onSelectSftpLink: (Long) -> Unit = {},
    onCloseSftpLink: (Long) -> Unit = {},
    quickKeyValues: List<String> = dev.akko.miaominal.data.DefaultTerminalQuickKeys,
    modifier: Modifier = Modifier
) {
    val sessionLinks = sessionTabs.map { tab ->
        OpenedLinkUi(
            id = "ssh:${tab.id}",
            connectionId = tab.id,
            title = tab.title,
            subtitle = tab.subtitle,
            kind = OpenedLinkKind.Ssh,
            status = when {
                tab.waiting -> OpenedLinkStatus.Idle
                tab.active -> OpenedLinkStatus.Active
                else -> OpenedLinkStatus.Connected
            }
        )
    } + sftpLinks + agentLinks
    var expanded by rememberSaveable { mutableStateOf(false) }
    var fontSize by rememberSaveable { mutableIntStateOf(14) }
    var showSnippets by remember { mutableStateOf(false) }
    var showIdentity by remember { mutableStateOf(false) }
    var moreActions by remember { mutableStateOf(false) }
    val explosion: TerminalTextExplosionViewModel = androidx.lifecycle.viewmodel.compose.viewModel(
        key = "terminal-explosion-${session?.connectionId}")
    var snapshot by remember(session?.connectionId) {
        mutableStateOf(session?.snapshot())
    }
    var showingHistory by remember(session?.connectionId) { mutableStateOf(false) }
    var surfaceView by remember { mutableStateOf<TerminalSurfaceView?>(null) }
    val resizeQueue = remember(session) { TerminalResizeQueue() }
    var drawerOpen by rememberSaveable { mutableStateOf(false) }
    val density = LocalDensity.current
    val followSystemFont = LocalUiPreferences.current?.followTerminalFontScale ?: false
    val narrow = dev.akko.miaominal.ui.components.LocalWorkspaceWidth.current < 400 || density.fontScale > 1.3f
    val activeConnectionId = session?.connectionId ?: sessionTabs.firstOrNull()?.id
    val normalizedQuickKeys = remember(quickKeyValues) { normalizeTerminalQuickKeys(quickKeyValues) }
    val quickKeys = remember(normalizedQuickKeys) {
        normalizedQuickKeys.mapNotNull { value -> AllTerminalKeys.firstOrNull { it.value == value } }
    }
    val moreKeys = remember(normalizedQuickKeys) {
        val selected = normalizedQuickKeys.toSet()
        AllTerminalKeys.filterNot { it.value in selected }
    }

    DisposableEffect(resizeQueue) {
        onDispose { resizeQueue.close() }
    }
    LaunchedEffect(resizeQueue, session) {
        val current = session ?: return@LaunchedEffect
        withContext(Dispatchers.Default) {
            resizeQueue.consume { columns, rows -> current.resize(columns, rows) }
        }
    }

    // Long-poll while idle and coalesce live output at the display refresh rate.
    LaunchedEffect(session, surfaceView) {
        val current = session ?: return@LaunchedEffect
        val view = surfaceView ?: return@LaunchedEffect
        pumpTerminalFrames(current,
            awaitDisplayFrame = { withFrameNanos { } },
            onFrame = { frame ->
                if (view.session === current) {
                    view.applyFrame(frame)
                    // The button needs a boolean, not a recomposition per scrolled row.
                    showingHistory = frame.displayOffset > 0
                }
            },
            onSnapshot = { state ->
                if (view.session === current && (state.phase != snapshot?.phase || state.failure != snapshot?.failure)) snapshot = state
            })
    }

    DisposableEffect(session?.connectionId) {
        val current = session
        current?.focused = true
        surfaceView?.apply {
            this.session = current
            inputEnabled = snapshot?.phase == SessionPhase.Ready
            // The new session gets its current grid through its own resize queue.
            notifyGeometry()
        }
        onDispose { current?.focused = false }
    }

    LaunchedEffect(surfaceView, fontSize, density, followSystemFont) {
        surfaceView?.setTextSizePx(fontSize * density.density * dev.akko.miaominal.ui.theme.terminalFontScale(followSystemFont, density.fontScale))
    }

    val phase = snapshot?.phase ?: SessionPhase.Closed
    val connected = phase == SessionPhase.Ready
    LaunchedEffect(surfaceView, connected) {
        surfaceView?.inputEnabled = connected
        if (connected) surfaceView?.notifyGeometry()
    }
    val keyboardVisible = WindowInsets.ime.getBottom(density) > 0

    BackHandler {
        when {
            drawerOpen -> drawerOpen = false
            keyboardVisible -> surfaceView?.clearFocus()
            expanded -> expanded = false
            else -> onDismiss()
        }
    }

    SessionDrawer(
        open = drawerOpen,
        onClose = { drawerOpen = false },
        modifier = modifier,
        drawerContent = {
            OpenedLinksDrawer(
                links = sessionLinks,
                selectedLinkId = activeConnectionId?.let { "ssh:$it" },
                onClose = { drawerOpen = false },
                onSelect = { link ->
                    drawerOpen = false
                    when (link.kind) {
                        OpenedLinkKind.Agent -> onSelectAgent()
                        OpenedLinkKind.Ssh -> onSwitchSession(link.connectionId)
                        OpenedLinkKind.Sftp -> onSelectSftpLink(link.connectionId)
                    }
                },
                onCloseLink = { link ->
                    drawerOpen = false
                    when (link.kind) {
                        OpenedLinkKind.Agent -> onCloseAgent()
                        OpenedLinkKind.Ssh -> onCloseSession(link.connectionId)
                        OpenedLinkKind.Sftp -> onCloseSftpLink(link.connectionId)
                    }
                }
            )
        }
    ) {
        Box(
            Modifier.fillMaxSize()
                .background(MaterialTheme.colorScheme.surface)
                .navigationBarsPadding()
                .imePadding()
        ) {
        Column(
            Modifier.fillMaxSize()
                .statusBarsPadding()
                .padding(top = SessionTopBarHeight)
        ) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                AndroidView(
                    modifier = Modifier.fillMaxSize().testTag("terminal-surface"),
                    factory = { context ->
                        TerminalSurfaceView(context).also { view ->
                            view.session = session
                            view.inputEnabled = connected
                            view.onGeometryChanged = { columns, rows ->
                                resizeQueue.request(columns, rows)
                            }
                            view.onExplodeSelection = explosion::open
                            surfaceView = view
                        }
                    },
                    update = { view ->
                        view.session = session
                        view.inputEnabled = connected
                        view.onGeometryChanged = { columns, rows -> resizeQueue.request(columns, rows) }
                        view.onExplodeSelection = explosion::open
                    }
                )
                if (showingHistory) {
                    FilledTonalButton(
                        onClick = { session?.scrollTo(0) },
                        modifier = Modifier.align(Alignment.BottomEnd).padding(12.dp)
                    ) {
                        Icon(Icons.Outlined.ArrowDownward, contentDescription = null, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(stringResource(R.string.term_scroll_bottom))
                    }
                }
            }
            if (connected) {
                TerminalKeyDock(expanded, fontSize, quickKeys, moreKeys,
                    onKey = { key ->
                        if (key == "SMALLER") fontSize = (fontSize - 1).coerceAtLeast(11)
                        else if (key == "LARGER") fontSize = (fontSize + 1).coerceAtMost(22)
                        else sendDockKey(session, key)
                    },
                    onExpand = { expanded = !expanded }
                )
            } else {
                Row(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer)
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        when (phase) {
                            SessionPhase.Failed -> snapshot?.failure
                                ?: stringResource(R.string.connection_failed)
                            SessionPhase.Closing, SessionPhase.Closed ->
                                stringResource(R.string.terminal_session_ended)
                            else -> stringResource(R.string.connection_waiting_auth)
                        },
                        color = if (phase == SessionPhase.Failed) MaterialTheme.colorScheme.error
                        else MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f), style = MaterialTheme.typography.bodySmall
                    )
                    TextButton(onClick = { activeConnectionId?.let(onCloseSession) }) {
                        Text(stringResource(R.string.terminal_close_session))
                    }
                }
                ConnectionErrorActions(snapshot?.failure.orEmpty(), onReconnect, onEditProfile)
            }
        }
        // Drawn after the terminal surface: interop views are composited above
        // the Compose content that precedes them, so a top bar composed before
        // the AndroidView would be covered by it.
        Column(
            Modifier
                .fillMaxWidth()
                .align(Alignment.TopCenter)
                .statusBarsPadding()
        ) {
            SessionTopBarCapsule {
                IconButton(onClick = { surfaceView?.clearFocus(); drawerOpen = true }) {
                    Icon(Icons.Outlined.Menu, stringResource(R.string.links_sidebar_toggle),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                SessionIdentity(profile, connected, Modifier.weight(1f).clickable { showIdentity = true })
                if (!narrow) onOpenAgent?.let { openAgent ->
                    IconButton(onClick = { surfaceView?.clearFocus(); openAgent() }) {
                        Icon(Icons.Outlined.SmartToy, stringResource(R.string.terminal_open_agent),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                if (!narrow) IconButton(onClick = { surfaceView?.clearFocus(); showSnippets = true }, enabled = connected) {
                    Icon(Icons.Outlined.Code, stringResource(R.string.terminal_snippets),
                        tint = if (connected) MaterialTheme.colorScheme.onSurfaceVariant
                        else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f))
                }
                if (narrow) Box {
                    IconButton(onClick = { surfaceView?.clearFocus(); moreActions = true }) {
                        Icon(Icons.Outlined.MoreVert, stringResource(R.string.fm_more_actions))
                    }
                    DropdownMenu(moreActions, { moreActions = false }) {
                        onOpenAgent?.let { open -> DropdownMenuItem(text = { Text(stringResource(R.string.terminal_open_agent)) },
                            onClick = { moreActions = false; open() }) }
                        DropdownMenuItem(text = { Text(stringResource(R.string.terminal_snippets)) }, enabled = connected,
                            onClick = { moreActions = false; showSnippets = true })
                    }
                }
                IconButton(onClick = { surfaceView?.clearFocus(); onOpenSftp() }) {
                    Icon(Icons.Outlined.FolderOpen, stringResource(R.string.terminal_open_sftp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
        }
    }
    if (showIdentity) AlertDialog(onDismissRequest = { showIdentity = false },
        title = { Text(profile.name) },
        text = { SelectionContainer { Text("${profile.username}@${profile.address}:${profile.port}") } },
        confirmButton = { TextButton(onClick = { showIdentity = false }) { Text(stringResource(android.R.string.ok)) } })
    if (showSnippets) TerminalSnippetsSheet(
        onDismiss = { showSnippets = false },
        onSelect = { snippet ->
            surfaceView?.typeText(snippet.command)
            showSnippets = false
        }
    )
    if (explosion.source != null) TerminalTextExplosionDialog(explosion.document, onDismiss = explosion::close)
}


private fun sendDockKey(
    session: TerminalSessionHandle?,
    key: String
) {
    if (session == null) return
    // A disconnect can race the dock: never let a rejected write crash the app.
    runCatching {
        when (key) {
            "ESC" -> session.handleKey("escape", null, false, false, false)
            "TAB" -> session.handleKey("tab", null, false, false, false)
            "INTERRUPT" -> session.handleKey("c", null, true, false, false)
            "UP" -> session.handleKey("up", null, false, false, false)
            "DOWN" -> session.handleKey("down", null, false, false, false)
            "LEFT" -> session.handleKey("left", null, false, false, false)
            "RIGHT" -> session.handleKey("right", null, false, false, false)
            "HOME" -> session.handleKey("home", null, false, false, false)
            "END" -> session.handleKey("end", null, false, false, false)
            "PAGEUP" -> session.handleKey("pageup", null, false, false, false)
            "PAGEDOWN" -> session.handleKey("pagedown", null, false, false, false)
            "ERASE" -> session.handleKey("u", null, true, false, false)
            "CLEAR" -> session.handleKey("l", null, true, false, false)
            // Symbols go straight to the PTY; popping the IME for one character
            // is disruptive when a hardware keyboard is attached.
            "/", "|", "~", "-", "*", "!" -> session.writeBytes(key.toByteArray())
        }
    }
}

@Composable
private fun TerminalKeyDock(
    expanded: Boolean,
    fontSize: Int,
    quickKeys: List<TerminalKey>,
    moreKeys: List<TerminalKey>,
    onKey: (String) -> Unit,
    onExpand: () -> Unit
) {
    Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer)) {
        AnimatedVisibility(expanded) {
            Column(Modifier.padding(top = 4.dp)) {
                moreKeys.chunked(8).forEach { keys ->
                    TerminalKeyRow(keys, fontSize, onKey)
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 6.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            TerminalKeyRow(quickKeys, fontSize, onKey, Modifier.weight(1f))
            if (moreKeys.isNotEmpty()) {
                IconButton(onClick = onExpand) {
                    Icon(if (expanded) Icons.Outlined.ExpandMore else Icons.Outlined.ExpandLess,
                        stringResource(if (expanded) R.string.term_keys_less else R.string.term_keys_more),
                        tint = if (expanded) MaterialTheme.colorScheme.primary
                        else MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
    }
}

@Composable
private fun TerminalKeyRow(
    keys: List<TerminalKey>,
    fontSize: Int,
    onKey: (String) -> Unit,
    modifier: Modifier = Modifier
) {
    LazyRow(
        modifier = modifier.padding(horizontal = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(2.dp)
    ) {
        items(keys, key = { it.value }) { key ->
            val repeat: (() -> Unit)? =
                if (key.value in RepeatableKeys) ({ onKey(key.value) }) else null
            TerminalKeyButton(
                key = key,
                enabled = when (key.value) {
                    "SMALLER" -> fontSize > 11
                    "LARGER" -> fontSize < 22
                    else -> true
                },
                onRepeat = repeat,
                onClick = { onKey(key.value) }
            )
        }
    }
}

@Composable
private fun TerminalKeyButton(
    key: TerminalKey,
    enabled: Boolean = true,
    onRepeat: (() -> Unit)? = null,
    onClick: () -> Unit
) {
    val description = stringResource(key.description)
    val interactionSource = remember { MutableInteractionSource() }
    val scope = rememberCoroutineScope()
    // The gesture below only restarts when `enabled`/`onRepeat` change, so the
    // latest handlers stay reachable without restarting it.
    val currentClick by rememberUpdatedState(onClick)
    val currentRepeat by rememberUpdatedState(onRepeat)
    Surface(
        modifier = Modifier
            .size(48.dp)
            .semantics {
                role = Role.Button
                contentDescription = description
                if (enabled) {
                    this.onClick { currentClick(); true }
                } else {
                    disabled()
                }
            }
            .indication(interactionSource, LocalIndication.current)
            .pointerInput(enabled, onRepeat != null) {
                if (!enabled) return@pointerInput
                awaitEachGesture {
                    val down = awaitFirstDown()
                    val press = PressInteraction.Press(down.position)
                    // The pointer scope is a restricted suspend scope, so the
                    // ripple interactions are emitted from the composition scope.
                    scope.launch { interactionSource.emit(press) }
                    // Send on press so the key reacts immediately; hold-to-repeat
                    // keys keep sending until the finger lifts or drags away
                    // (a drag scrolls the key row instead of repeating).
                    currentClick()
                    val startMs = SystemClock.uptimeMillis()
                    var nextRepeatMs = KeyRepeatStartDelayMs
                    var canceled = false
                    while (true) {
                        val event = withTimeoutOrNull(KeyRepeatPollMs) { awaitPointerEvent() }
                        if (event != null) {
                            val change = event.changes.firstOrNull { it.id == down.id }
                            if (change == null || !change.pressed) break
                            if ((change.position - down.position).getDistance() > viewConfiguration.touchSlop) {
                                canceled = true
                                break
                            }
                        }
                        val repeat = currentRepeat ?: continue
                        val heldMs = SystemClock.uptimeMillis() - startMs
                        if (heldMs >= nextRepeatMs) {
                            repeat()
                            nextRepeatMs = heldMs + KeyRepeatIntervalMs
                        }
                    }
                    scope.launch {
                        interactionSource.emit(
                            if (canceled) PressInteraction.Cancel(press)
                            else PressInteraction.Release(press)
                        )
                    }
                }
            }
            .padding(vertical = 5.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHighest,
        contentColor = if (enabled) MaterialTheme.colorScheme.onSurface
        else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f),
        shape = RoundedCornerShape(6.dp)
    ) {
        Box(contentAlignment = Alignment.Center) {
            Text(key.label, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TerminalSnippetsSheet(
    onDismiss: () -> Unit,
    onSelect: (TerminalSnippet) -> Unit
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surfaceContainer
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(bottom = 16.dp)
        ) {
            Text(
                text = stringResource(R.string.terminal_snippets_title),
                modifier = Modifier.padding(horizontal = 24.dp),
                style = MaterialTheme.typography.titleLarge,
                color = MaterialTheme.colorScheme.onSurface
            )
            Text(
                text = stringResource(R.string.terminal_snippets_subtitle),
                modifier = Modifier.padding(start = 24.dp, top = 4.dp, end = 24.dp, bottom = 12.dp),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            TerminalSnippets.forEachIndexed { index, snippet ->
                ListItem(
                    headlineContent = {
                        Text(stringResource(snippet.labelRes))
                    },
                    supportingContent = {
                        Text(
                            text = snippet.command,
                            style = MaterialTheme.typography.bodyMedium.copy(
                                fontFamily = FontFamily.Monospace
                            )
                        )
                    },
                    leadingContent = {
                        Icon(Icons.Outlined.Terminal, contentDescription = null)
                    },
                    modifier = Modifier.clickable { onSelect(snippet) },
                    colors = ListItemDefaults.colors(
                        containerColor = Color.Transparent,
                        headlineColor = MaterialTheme.colorScheme.onSurface,
                        supportingColor = MaterialTheme.colorScheme.onSurfaceVariant,
                        leadingIconColor = MaterialTheme.colorScheme.primary
                    )
                )
                if (index != TerminalSnippets.lastIndex) {
                    HorizontalDivider(
                        modifier = Modifier.padding(start = 72.dp),
                        color = MaterialTheme.colorScheme.outlineVariant
                    )
                }
            }
        }
    }
}
