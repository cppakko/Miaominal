package dev.akko.miaominal.ui

import dev.akko.miaominal.ui.theme.UiSpacing
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import dev.akko.miaominal.ui.components.AdaptiveWorkspace
import dev.akko.miaominal.ui.components.AnimatedDetailPane
import dev.akko.miaominal.ui.components.LocalTwoPaneAvailable
import dev.akko.miaominal.ui.components.LocalBackGesture
import dev.akko.miaominal.ui.components.BackGestureState
import dev.akko.miaominal.ui.components.HostRow
import dev.akko.miaominal.ui.model.AppNavigationState

import dev.akko.miaominal.ui.uiString

import android.app.Activity
import androidx.annotation.StringRes
import dev.akko.miaominal.ui.components.WorkspaceBackHandler as BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.ContentTransform
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.updateTransition
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Widgets
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Widgets
import androidx.compose.material3.Badge
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.windowsizeclass.ExperimentalMaterial3WindowSizeClassApi
import androidx.compose.material3.windowsizeclass.WindowSizeClass
import androidx.compose.material3.windowsizeclass.WindowWidthSizeClass
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.hideFromAccessibility
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.DpSize
import androidx.core.view.WindowCompat
import dev.akko.miaominal.R
import dev.akko.miaominal.data.CredentialUpdate
import dev.akko.miaominal.data.HostRuntimeStatus
import dev.akko.miaominal.data.MiaominalRepository
import dev.akko.miaominal.data.SessionPhase
import dev.akko.miaominal.data.ForwardPhaseModel
import dev.akko.miaominal.data.SftpPhase
import dev.akko.miaominal.data.agent.AgentChatRepository
import dev.akko.miaominal.data.agent.AgentInteractionAnswer
import dev.akko.miaominal.data.agent.AgentSessionState
import dev.akko.miaominal.data.agent.InteractionAnswerEntry
import dev.akko.miaominal.ui.components.OpenedLinksDrawer
import dev.akko.miaominal.ui.components.OpenedLinkKind
import dev.akko.miaominal.ui.components.OpenedLinkStatus
import dev.akko.miaominal.ui.components.OpenedLinkUi
import dev.akko.miaominal.ui.components.SessionDrawer
import dev.akko.miaominal.ui.components.SessionRequestDialog
import dev.akko.miaominal.ui.model.HostProfileUi
import dev.akko.miaominal.ui.model.toDomain
import dev.akko.miaominal.ui.model.toUi
import dev.akko.miaominal.ui.screens.HostSearchTopBar
import dev.akko.miaominal.ui.screens.AgentChatScreen
import dev.akko.miaominal.ui.screens.AgentLaunchSheet
import dev.akko.miaominal.ui.screens.ConnectingScreen
import dev.akko.miaominal.ui.screens.HostsScreen
import dev.akko.miaominal.ui.screens.OnboardingScreen
import dev.akko.miaominal.ui.screens.ProfileEditorScreen
import dev.akko.miaominal.ui.screens.SettingsDestination
import dev.akko.miaominal.ui.screens.SettingsDetailScreen
import dev.akko.miaominal.ui.screens.SettingsScreen
import dev.akko.miaominal.ui.screens.SettingsSearchTopBar
import dev.akko.miaominal.ui.screens.SftpScreen
import dev.akko.miaominal.ui.screens.TerminalScreen
import dev.akko.miaominal.ui.screens.TerminalSessionTab
import dev.akko.miaominal.ui.screens.ToolDestination
import dev.akko.miaominal.ui.screens.ToolDetailScreen
import dev.akko.miaominal.ui.screens.ToolsScreen
import dev.akko.miaominal.ui.screens.ToolsSearchTopBar
import dev.akko.miaominal.ui.theme.MiaominalMotion
import dev.akko.miaominal.ui.theme.ThemeMode
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.delay

private data class ConnectionPresentation(
    val profile: HostProfileUi,
    val sftp: Boolean,
    val stage: Int,
    val phase: SessionPhase?,
    val failure: String?,
    val requestKind: dev.akko.miaominal.data.SessionRequestKind?
)

/**
 * Material 3 fade through for top-level destinations: the outgoing content
 * leaves first and the incoming content follows, so the two never blend. The
 * chrome (search capsule / top bar, bottom navigation) is not part of a spatial
 * transition, which is why tab changes do not scale.
 */
private fun fadeThrough(): ContentTransform =
    fadeIn(MiaominalMotion.fadeThroughIn()) togetherWith fadeOut(MiaominalMotion.fadeThroughOut())

/**
 * Crossfade for the persistent top bar while the pages below use the system
 * surface transition. Every tab draws the same capsule, so only the capsule
 * contents blend into each other instead of the bar blinking out.
 */
private fun chromeCrossFade(): ContentTransform =
    fadeIn(MiaominalMotion.chromeFadeIn()) togetherWith fadeOut(MiaominalMotion.chromeFadeOut())

/**
 * Hosts one full-screen session surface (SFTP, AI chat) above the base layer.
 *
 * The outgoing surface stays composed for one exit fade, so closing a session
 * animates instead of snapping back to the page below. Every session surface
 * draws the same capsule top bar at the same bounds, and because the system
 * surface transition never shows both surfaces at once, that shared chrome
 * reads as one element swapping its contents.
 */
@Composable
private fun <T : Any> SessionSurface(
    state: T?,
    contentKey: (T?) -> Any? = { it },
    content: @Composable (T) -> Unit
) {
    AnimatedContent(
        targetState = state,
        contentKey = contentKey,
        transitionSpec = { MiaominalMotion.surfaceTransition() },
        label = "session-surface",
        modifier = Modifier.fillMaxSize()
    ) { surface ->
        if (surface != null) content(surface)
    }
}

private enum class MainDestination(
    @param:StringRes val labelRes: Int,
    val selectedIcon: ImageVector,
    val unselectedIcon: ImageVector
) {
    Hosts(R.string.nav_hosts, Icons.Filled.Computer, Icons.Outlined.Computer),
    Tools(R.string.nav_tools, Icons.Filled.Widgets, Icons.Outlined.Widgets),
    Settings(R.string.nav_settings, Icons.Filled.Settings, Icons.Outlined.Settings)
}

private sealed interface AppLayer {
    data object Main : AppLayer
    data class Editor(val initial: HostProfileUi?) : AppLayer
    data class Terminal(val connectionId: Long, val profile: HostProfileUi) : AppLayer
    data class Tool(val destination: ToolDestination) : AppLayer
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MiaominalApp(
    repository: MiaominalRepository,
    themeMode: ThemeMode,
    darkTheme: Boolean,
    dynamicColor: Boolean,
    seedColor: Int,
    onThemeModeChange: (ThemeMode) -> Unit,
    onDynamicColorChange: (Boolean) -> Unit,
    onSeedColorChange: (Int) -> Unit,
    agentChat: AgentChatRepository? = null
) {
    val backGesture = remember { BackGestureState() }
    CompositionLocalProvider(LocalMiaominalRepository provides repository, LocalBackGesture provides backGesture) {
        AdaptiveWorkspace {
        MiaominalAppContent(
            repository = repository,
            themeMode = themeMode,
            darkTheme = darkTheme,
            dynamicColor = dynamicColor,
            seedColor = seedColor,
            onThemeModeChange = onThemeModeChange,
            onDynamicColorChange = onDynamicColorChange,
            onSeedColorChange = onSeedColorChange,
            agentChat = agentChat
        )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3WindowSizeClassApi::class)
@Composable
private fun MiaominalAppContent(
    repository: MiaominalRepository,
    themeMode: ThemeMode,
    darkTheme: Boolean,
    dynamicColor: Boolean,
    seedColor: Int,
    onThemeModeChange: (ThemeMode) -> Unit,
    onDynamicColorChange: (Boolean) -> Unit,
    onSeedColorChange: (Int) -> Unit,
    agentChat: AgentChatRepository?
) {
    val navigation = rememberSaveable(saver = AppNavigationState.Saver) { AppNavigationState() }
    var selectedName by navigation::selectedName
    val snackbar = remember { SnackbarHostState() }
    val tabStates = rememberSaveableStateHolder()
    val detailStates = rememberSaveableStateHolder()
    val sessionStates = rememberSaveableStateHolder()
    val twoPanes = LocalTwoPaneAvailable.current
    val haptic = androidx.compose.ui.platform.LocalHapticFeedback.current
    val backGesture = LocalBackGesture.current
    val selected = MainDestination.valueOf(selectedName)
    val resources = LocalResources.current
    val profiles by repository.hosts.collectAsState()
    val statuses by repository.hostStatus.collectAsState()
    val liveSessions by repository.sessions.collectAsState()
    val sftpSessions by repository.sftpSessions.collectAsState()
    val forwardSessions by repository.forwardSessions.collectAsState()
    val connectedHostIds = remember(liveSessions, sftpSessions, forwardSessions) {
        buildSet {
            liveSessions.filter { it.phase == SessionPhase.Ready }.forEach { add(it.hostId) }
            sftpSessions.filter { it.phase == SftpPhase.Ready }.forEach { add(it.hostId) }
            forwardSessions.filter {
                it.phase == ForwardPhaseModel.Listening || it.phase == ForwardPhaseModel.Connected
            }.forEach { add(it.profileId) }
        }
    }
    val preferences by repository.settings.collectAsState()
    val devicePreferences by repository.devicePreferences.collectAsState()
    val managedKeys by repository.managedKeys.collectAsState()
    val scope = rememberCoroutineScope()
    LaunchedEffect(repository) { repository.notices.collect { snackbar.showSnackbar(it) } }
    val hosts = remember(profiles, statuses, resources, connectedHostIds) {
        profiles.map { profile ->
            val status = statuses[profile.id] ?: HostRuntimeStatus()
            profile.toUi(status.copy(online = profile.id in connectedHostIds), resources)
        }
    }
    val recentHosts = remember(profiles, hosts, preferences.recentHostCount) {
        profiles
            .filter { it.lastConnectedAt != null }
            .sortedByDescending { it.lastConnectedAt }
            .take(preferences.recentHostCount)
            .mapNotNull { profile -> hosts.firstOrNull { it.id == profile.id } }
    }
    var editorForNewHost by navigation::editorForNewHost
    var editorProfileId by navigation::editorProfileId
    val editorInitial = editorProfileId?.let { id -> hosts.firstOrNull { it.id == id } }
    val editorRequested = editorForNewHost || editorProfileId != null
    var sftpOriginTerminalId by rememberSaveable { mutableStateOf<String?>(null) }
    var sftpAttachConnectionId by rememberSaveable { mutableStateOf<Long?>(null) }
    var sftpShownConnectionId by rememberSaveable { mutableStateOf<Long?>(null) }
    fun closeEditor() {
        editorForNewHost = false
        editorProfileId = null
    }
    var expandedHostId by rememberSaveable { mutableStateOf<String?>(null) }
    var hostDetailVisible by rememberSaveable { mutableStateOf(false) }
    val hostStates = rememberSaveableStateHolder()
    var activeConnectionId by rememberSaveable { mutableStateOf<Long?>(null) }
    var sftpProfileId by rememberSaveable { mutableStateOf<String?>(null) }
    var connectingProfileId by rememberSaveable { mutableStateOf<String?>(null) }
    var connectingConnectionId by rememberSaveable { mutableStateOf<Long?>(null) }
    var connectingSftp by rememberSaveable { mutableStateOf(false) }
    var connecting by rememberSaveable { mutableStateOf(false) }
    var connectionOpenError by remember { mutableStateOf<String?>(null) }
    var connectionStage by remember { mutableStateOf(0) }
    var connectionSequence by rememberSaveable { mutableStateOf(0) }
    val sessionProfiles = remember { mutableStateMapOf<Long, HostProfileUi>() }
    fun profileForConnection(connectionId: Long): HostProfileUi? {
        sessionProfiles[connectionId]?.let { return it }
        val snapshot = liveSessions.firstOrNull { it.connectionId == connectionId } ?: return null
        return hosts.firstOrNull { it.id == snapshot.hostId }
    }
    fun openConnection(profile: HostProfileUi, sftp: Boolean) {
        connectionOpenError = null
        connectingProfileId = profile.id
        connectingSftp = sftp
        connectionStage = 0
        connectionSequence += 1
        val sequence = connectionSequence
        connecting = true
        if (sftp) {
            // The SFTP screen opens its own real session and reports the typed
            // connection phase; there is no staged demo timeline any more.
            connectingConnectionId = null
            sftpOriginTerminalId = activeConnectionId?.toString()
            sftpAttachConnectionId = null
            sftpShownConnectionId = null
            sftpProfileId = profile.id
            connecting = false
            connectingProfileId = null
            return
        }
        connectingConnectionId = null
        scope.launch {
            val columns = 120
            val rows = 32
            repository.openSession(profile.id, columns, rows)
                .onSuccess { handle ->
                    if (sequence != connectionSequence || !connecting) {
                        repository.closeSession(handle.connectionId)
                        return@onSuccess
                    }
                    sessionProfiles[handle.connectionId] = profile
                    connectingConnectionId = handle.connectionId
                }
                .onFailure { if (sequence == connectionSequence && connecting) connectionOpenError = it.message ?: uiString(R.string.ux_failed) }
        }
    }
    fun performCloseSession(connectionId: Long) {
        scope.launch {
            repository.closeSession(connectionId).onSuccess {
                if (activeConnectionId == connectionId) activeConnectionId = null
                sessionProfiles.remove(connectionId)
                if (connectingConnectionId == connectionId) {
                    connecting = false
                    connectingProfileId = null
                    connectingConnectionId = null
                }
            }.onFailure { snackbar.showSnackbar(it.message ?: uiString(R.string.ux_failed)) }
        }
    }
    fun closeSession(connectionId: Long) {
        if (dev.akko.miaominal.ui.model.requiresSshCloseConfirmation(liveSessions.firstOrNull { it.connectionId == connectionId }?.phase))
            navigation.pendingClose = "ssh:$connectionId"
        else performCloseSession(connectionId)
    }
    fun performSftpClose(connectionId: Long) {
        scope.launch {
            repository.closeSftpSession(connectionId).onSuccess {
                if (sftpShownConnectionId == connectionId) {
                    sftpProfileId = null; sftpAttachConnectionId = null; sftpShownConnectionId = null
                }
            }.onFailure { snackbar.showSnackbar(it.message ?: uiString(R.string.ux_failed)) }
        }
    }
    fun requestSftpClose(connectionId: Long) {
        val tasks = repository.sftpSessionHandle(connectionId)?.transfersSnapshot().orEmpty()
        if (tasks.any { !it.state.isTerminal }) navigation.pendingClose = "sftp:$connectionId"
        else performSftpClose(connectionId)
    }
    fun cancelConnection() {
        connectionSequence += 1
        val id = connectingConnectionId
        val wasSftp = connectingSftp
        connecting = false
        connectingSftp = false
        connectingProfileId = null
        connectingConnectionId = null
        if (!wasSftp && id != null) {
            scope.launch { repository.closeSession(id) }
        }
    }
    // Follow the real session phases; the SFTP demo keeps its staged timeline.
    LaunchedEffect(connecting, connectionSequence, connectingConnectionId) {
        if (!connecting || connectingSftp) return@LaunchedEffect
        val id = connectingConnectionId ?: return@LaunchedEffect
        while (isActive) {
            val snapshot = liveSessions.firstOrNull { it.connectionId == id }
            when (snapshot?.phase) {
                SessionPhase.Ready -> {
                    activeConnectionId = id
                    hosts.firstOrNull { it.id == snapshot.hostId }?.let { host ->
                        sessionProfiles.putIfAbsent(id, host)
                    }
                    connecting = false
                    connectingProfileId = null
                    connectingConnectionId = null
                    return@LaunchedEffect
                }
                SessionPhase.Failed, SessionPhase.Closed -> return@LaunchedEffect
                else -> delay(120)
            }
        }
    }
    var selectedToolName by navigation::selectedToolName
    // Query carried from the tools search into the tool page it opened.
    var toolSearchQuery by rememberSaveable { mutableStateOf("") }
    var selectedSettingName by navigation::selectedSettingName
    // Section carried from the settings search into the page it opened.
    var settingFocusAnchor by rememberSaveable { mutableStateOf<String?>(null) }
    // AI chat overlays the terminal layer, so terminal sessions and their
    // scrollback survive while the chat page is open.
    var agentSheetHostId by navigation::agentSheetHostId
    var agentChatVisible by navigation::agentChatVisible
    val agentState = agentChat?.state?.collectAsState()?.value ?: AgentSessionState()
    val agentDraft = agentChat?.draft?.collectAsState()?.value.orEmpty()
    val agentFiles = agentChat?.files?.collectAsState()?.value.orEmpty()
    var agentFilePickerOpen by rememberSaveable { mutableStateOf(false) }
    var confirmEndAgent by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(agentState.session) {
        // Android may restore the Activity's page flag after the process died.
        // There is no local conversation to show on a cold start anymore.
        if (agentState.session == null) {
            agentChatVisible = false
            agentFilePickerOpen = false
            confirmEndAgent = false
        }
    }

    val availableGroups = remember(hosts) {
        hosts.map { it.group }.filter { it.isNotBlank() }.distinct().sorted()
    }
    val selectedTool = selectedToolName?.let(ToolDestination::valueOf)
    val selectedSetting = selectedSettingName?.let {
        if (settingFocusAnchor == "appearance-typography") SettingsDestination.TerminalKeys
        else SettingsDestination.valueOf(it)
    }
    val terminalProfile = activeConnectionId?.let(::profileForConnection)
    val sftpProfile = hosts.firstOrNull { it.id == sftpProfileId }
    val sessionTabs = liveSessions.map { snapshot ->
        TerminalSessionTab(
            id = snapshot.connectionId,
            title = profileForConnection(snapshot.connectionId)?.name ?: snapshot.hostLabel,
            subtitle = hosts.firstOrNull { it.id == snapshot.hostId }?.let {
                "${it.username}@${it.address}:${it.port}"
            } ?: snapshot.hopLabel.orEmpty(),
            active = snapshot.connectionId == activeConnectionId,
            waiting = !snapshot.phase.isTerminal && snapshot.phase != SessionPhase.Ready
        )
    }
    // SFTP sessions are first-class links too: they live in the same drawer as
    // the SSH sessions, and an entry can reopen the SFTP surface or close it.
    val sftpLinks = sftpSessions.filter { !it.phase.isTerminal }.map { sftp ->
        val profile = hosts.firstOrNull { it.id == sftp.hostId }
        OpenedLinkUi(
            id = "sftp:${sftp.connectionId}",
            connectionId = sftp.connectionId,
            title = profile?.name ?: sftp.hostLabel,
            subtitle = profile?.let { "${it.username}@${it.address}:${it.port}" }
                ?: sftp.hopLabel.orEmpty(),
            kind = OpenedLinkKind.Sftp,
            status = when {
                sftp.activeTransferCount > 0 -> OpenedLinkStatus.Active
                sftp.phase == SftpPhase.Ready -> OpenedLinkStatus.Connected
                sftp.phase == SftpPhase.Failed -> OpenedLinkStatus.Error
                else -> OpenedLinkStatus.Idle
            }
        )
    }
    val sshLinks = liveSessions.map { snapshot ->
        val profile = profileForConnection(snapshot.connectionId)
        OpenedLinkUi(
            id = "ssh:${snapshot.connectionId}",
            connectionId = snapshot.connectionId,
            title = profile?.name ?: snapshot.hostLabel,
            subtitle = profile?.let { "${it.username}@${it.address}:${it.port}" }
                ?: snapshot.hopLabel.orEmpty(),
            kind = OpenedLinkKind.Ssh,
            status = when {
                snapshot.request != null -> OpenedLinkStatus.Idle
                snapshot.phase == SessionPhase.Ready &&
                    snapshot.connectionId == activeConnectionId ->
                    OpenedLinkStatus.Active
                snapshot.phase == SessionPhase.Ready -> OpenedLinkStatus.Connected
                snapshot.phase.isTerminal -> OpenedLinkStatus.Error
                else -> OpenedLinkStatus.Idle
            }
        )
    }
    val agentLinks = agentState.session?.let { session ->
        listOf(OpenedLinkUi(
            id = "agent:current", connectionId = 0L,
            title = hosts.firstOrNull { it.id == session.hostId }?.name ?: session.hostId,
            subtitle = "${session.cli.displayName} · ${session.cwd}",
            kind = OpenedLinkKind.Agent,
            status = when {
                agentState.connectionLost -> OpenedLinkStatus.Error
                agentState.isBusy -> OpenedLinkStatus.Active
                else -> OpenedLinkStatus.Connected
            }
        ))
    }.orEmpty()
    val appLayer = when {
        editorRequested -> AppLayer.Editor(editorInitial)
        terminalProfile != null && activeConnectionId != null ->
            AppLayer.Terminal(activeConnectionId!!, terminalProfile)
        selectedTool != null -> AppLayer.Tool(selectedTool)
        else -> AppLayer.Main
    }
    val settingDetailContent: @Composable (SettingsDestination) -> Unit = { detail ->
        detailStates.SaveableStateProvider("setting:${detail.name}") {
            SettingsDetailScreen(
                destination = detail,
                onManageProxies = { selectedToolName = ToolDestination.Proxies.name },
                themeMode = themeMode,
                dynamicColor = dynamicColor,
                seedColor = seedColor,
                onThemeModeChange = onThemeModeChange,
                onDynamicColorChange = onDynamicColorChange,
                onSeedColorChange = onSeedColorChange,
                onReopenOnboarding = { scope.launch { repository.updateSettings { it.copy(onboardingCompleted = false) } } },
                onDismiss = { selectedSettingName = null; settingFocusAnchor = null },
                focusAnchor = settingFocusAnchor,
                modifier = Modifier.fillMaxSize().testTag("setting-detail:${detail.name}")
            )
        }
    }
    // Pending trust/authentication request from the active session (or any).
    val pendingRequestSession = liveSessions.firstOrNull {
        it.connectionId == activeConnectionId && it.request != null
    } ?: liveSessions.firstOrNull {
        it.connectionId == connectingConnectionId && it.request != null
    } ?: liveSessions.firstOrNull { it.request != null }
    val pendingRequest = pendingRequestSession?.request
    // Every layer follows the theme (terminal palette included), so the system
    // bar icons track the theme surface instead of a hardcoded terminal rule.
    val lightSystemBars = MaterialTheme.colorScheme.surface.luminance() > 0.5f
    SystemBarAppearance(light = lightSystemBars)

    BackHandler(
        enabled = editorRequested || terminalProfile != null ||
                selectedTool != null || selectedSetting != null || agentChatVisible || agentSheetHostId != null ||
                (selected == MainDestination.Hosts && hostDetailVisible && !twoPanes)
    ) {
        if (agentSheetHostId != null) {
            agentSheetHostId = null
        } else if (agentChatVisible) {
            agentChatVisible = false
            agentSheetHostId = agentState.session?.hostId
        } else if (editorRequested) {
            closeEditor()
        } else if (terminalProfile != null) {
            activeConnectionId = null
        } else if (selectedTool != null) {
            selectedToolName = null
        } else if (selected == MainDestination.Hosts && hostDetailVisible && !twoPanes) {
            hostDetailVisible = false
            expandedHostId = null
        } else {
            selectedSettingName = null
        }
    }

    // Retain a complete outgoing presentation until its fade has finished.
    val connectingSnapshot = liveSessions.firstOrNull { it.connectionId == connectingConnectionId }
    val connectionPresentation = if (connecting) {
        hosts.firstOrNull { it.id == connectingProfileId }?.let {
            ConnectionPresentation(it, connectingSftp, connectionStage,
                connectingSnapshot?.phase, connectionOpenError ?: connectingSnapshot?.failure,
                connectingSnapshot?.request?.kind)
        }
    } else null
    val connectionTransition = updateTransition(connectionPresentation, label = "connection-surface")

    // Opaque app surface behind every layer. Page transitions fade between two
    // semi-transparent surfaces, and the XML window background cannot follow
    // the in-app theme mode, so without this the window color flashes through
    // mid-transition.
    BoxWithConstraints(
        modifier = Modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.surface)
            .graphicsLayer {
                scaleX = 1f - backGesture.progress * 0.06f
                scaleY = scaleX
                translationX = backGesture.progress * size.width * 0.04f
            }
    ) {
        val windowSizeClass = WindowSizeClass.calculateFromSize(DpSize(maxWidth, maxHeight))
        val expanded = windowSizeClass.widthSizeClass != WindowWidthSizeClass.Compact

        AnimatedContent(
            targetState = appLayer,
            transitionSpec = {
                when {
                    // The retained connecting surface already fades away over
                    // this destination. Do not animate the terminal a second time.
                    targetState is AppLayer.Terminal && connectionTransition.currentState != null -> {
                        EnterTransition.None togetherWith ExitTransition.None
                    }
                    // Sessions and pushed pages are full-screen surfaces: they
                    // use the Android system scale + fade through so the two
                    // surfaces are never translucent at the same time.
                    else -> MiaominalMotion.surfaceTransition()
                }
            },
            label = "app-layer",
            modifier = Modifier.fillMaxSize()
        ) { layer ->
            when (layer) {
                is AppLayer.Editor -> ProfileEditorScreen(
                    initial = layer.initial,
                    availableGroups = availableGroups,
                    managedKeys = managedKeys,
                    hasStoredPassword = layer.initial?.id
                        ?.let { id -> profiles.firstOrNull { it.id == id }?.hasStoredPassword }
                        ?: false,
                    onSave = { saved, password ->
                        val credential = when {
                            password == null -> CredentialUpdate.Keep
                            password.isEmpty() -> CredentialUpdate.Clear
                            else -> CredentialUpdate.Set(password)
                        }
                        val result = repository.saveHost(saved.toDomain(), credential)
                        if (result.isSuccess) {
                            haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress)
                            expandedHostId = saved.id
                            closeEditor()
                            scope.launch { snackbar.showSnackbar(uiString(R.string.ux_saved)) }
                        }
                        result.map { }
                    },
                    onDismiss = { closeEditor() },
                    modifier = Modifier.fillMaxSize()
                )

                is AppLayer.Terminal -> TerminalScreen(
                    profile = layer.profile,
                    session = repository.sessionHandle(layer.connectionId),
                    sessionTabs = sessionTabs,
                    onSwitchSession = { target -> activeConnectionId = target },
                    onCloseSession = { target -> closeSession(target) },
                    onReconnect = { openConnection(layer.profile, sftp = false) },
                    onEditProfile = { editorProfileId = layer.profile.id },
                    onDismiss = { activeConnectionId = null },
                    onOpenSftp = {
                        openConnection(layer.profile, sftp = true)
                    },
                    onOpenAgent = if (agentChat != null) {
                        {
                            agentSheetHostId = layer.profile.id
                        }
                    } else {
                        null
                    },
                    sftpLinks = sftpLinks,
                    agentLinks = agentLinks,
                    onSelectAgent = { agentChatVisible = true },
                    onCloseAgent = { run { confirmEndAgent = true } },
                    onSelectSftpLink = { connectionId ->
                        sftpSessions.firstOrNull { it.connectionId == connectionId }?.let { sftp ->
                            sftpAttachConnectionId = sftp.connectionId
                            sftpShownConnectionId = sftp.connectionId
                            sftpOriginTerminalId = activeConnectionId?.toString()
                            sftpProfileId = sftp.hostId
                        }
                    },
                    onCloseSftpLink = { connectionId -> requestSftpClose(connectionId) },
                    quickKeyValues = devicePreferences.terminalQuickKeys,
                    modifier = Modifier.fillMaxSize()
                )

                is AppLayer.Tool -> ToolDetailScreen(
                    destination = layer.destination,
                    profiles = hosts,
                    onDismiss = {
                        selectedToolName = null
                        toolSearchQuery = ""
                    },
                    initialSearchQuery = toolSearchQuery,
                    modifier = Modifier.fillMaxSize()
                )

                AppLayer.Main -> {
                    var drawerOpen by rememberSaveable { mutableStateOf(false) }

                    Box(Modifier.fillMaxSize()) {
                    SessionDrawer(
                        open = drawerOpen,
                        onClose = { drawerOpen = false },
                        drawerContent = {
                            OpenedLinksDrawer(
                                links = sshLinks + sftpLinks + agentLinks,
                                selectedLinkId = activeConnectionId?.let { "ssh:$it" },
                                onClose = { drawerOpen = false },
                                onSelect = { link ->
                                    drawerOpen = false
                                    when (link.kind) {
                                        OpenedLinkKind.Agent -> { agentChatVisible = true }
                                        OpenedLinkKind.Ssh -> {
                                            if (liveSessions.any { it.connectionId == link.connectionId }) {
                                                activeConnectionId = link.connectionId
                                            }
                                        }

                                        OpenedLinkKind.Sftp -> {
                                            val sftp = sftpSessions.firstOrNull {
                                                it.connectionId == link.connectionId
                                            }
                                            if (sftp != null) {
                                                sftpAttachConnectionId = sftp.connectionId
                                                sftpShownConnectionId = sftp.connectionId
                                                sftpOriginTerminalId = activeConnectionId?.toString()
                                                sftpProfileId = sftp.hostId
                                            }
                                        }
                                    }
                                },
                                onCloseLink = { link ->
                                    drawerOpen = false
                                    when (link.kind) {
                                        OpenedLinkKind.Agent -> run { confirmEndAgent = true }
                                OpenedLinkKind.Ssh -> closeSession(link.connectionId)
                                        OpenedLinkKind.Sftp -> requestSftpClose(link.connectionId)
                                    }
                                }
                            )
                        },
                        modifier = Modifier.fillMaxSize().semantics {
                            if (!twoPanes && selected == MainDestination.Settings && selectedSetting != null) {
                                hideFromAccessibility()
                            }
                        }
                    ) {
                        Scaffold(
                            topBar = {
                                // Every tab uses the same capsule as the session
                                // pages, so switching tabs only swaps the capsule
                                // contents; the pages below use the system
                                // surface transition.
                                AnimatedContent(
                                    targetState = selected,
                                    transitionSpec = { chromeCrossFade() },
                                    label = "main-top-bar",
                                    modifier = Modifier.testTag("main-top-bar")
                                ) { destination ->
                                    tabStates.SaveableStateProvider("header:${destination.name}") {
                                    when (destination) {
                                        MainDestination.Hosts -> HostSearchTopBar(
                                            hosts = hosts,
                                            onConnectHost = { host -> openConnection(host, sftp = false) },
                                            onEditHost = { host ->
                                                editorProfileId = host.id
                                            },
                                            onAgentHost = if (agentChat != null) { { host -> agentChatVisible = false; agentSheetHostId = host.id } } else null,
                                            onSftpHost = { host -> openConnection(host, sftp = true) },
                                            onToggleFavorite = { host -> scope.launch { repository.toggleFavorite(host.id) } },
                                            onMenuClick = {
                                                drawerOpen = true
                                            }
                                        )

                                        MainDestination.Tools -> ToolsSearchTopBar(
                                            onToolClick = { tool, query ->
                                                toolSearchQuery = query
                                                selectedToolName = tool.name
                                            },
                                            onMenuClick = { drawerOpen = true }
                                        )

                                        MainDestination.Settings -> SettingsSearchTopBar(
                                            onSettingClick = { setting, anchor ->
                                                settingFocusAnchor = anchor
                                                selectedSettingName = setting.name
                                            },
                                            onMenuClick = { drawerOpen = true }
                                        )
                                    }
                                }
                                }
                            },
                            bottomBar = {
                                if (!expanded) {
                                    MiaominalNavigationBar(
                                        selected = selected,
                                        onSelected = { selectedName = it.name }
                                    )
                                }
                            },
                            floatingActionButton = {
                                if (selected == MainDestination.Hosts) {
                                    ExtendedFloatingActionButton(
                                        onClick = {
                                            editorForNewHost = true
                                        },
                                        containerColor = MaterialTheme.colorScheme.primaryContainer,
                                        contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                                        shape = androidx.compose.foundation.shape.CircleShape
                                    ) {
                                        Icon(Icons.Outlined.Add, null)
                                        Spacer(Modifier.width(8.dp))
                                        Text(stringResource(R.string.action_add_host))
                                    }
                                }
                            },
                            containerColor = MaterialTheme.colorScheme.surfaceContainer
                        ) { contentPadding ->
                            Row(
                                modifier = Modifier
                                    .fillMaxSize()
                                    .padding(contentPadding)
                                    .consumeWindowInsets(contentPadding)
                            ) {
                                if (expanded) {
                                    MiaominalNavigationRail(
                                        selected = selected,
                                        onSelected = { selectedName = it.name }
                                    )
                                    VerticalDivider(
                                        color = MaterialTheme.colorScheme.outlineVariant
                                    )
                                }

                                AnimatedContent(
                                    targetState = selected,
                                    transitionSpec = { fadeThrough().using(null) },
                                    label = "main-destination",
                                    modifier = Modifier.weight(1f).fillMaxSize().testTag("main-content")
                                ) { destination ->
                                    tabStates.SaveableStateProvider(destination.name) {
                                    when (destination) {
                                        MainDestination.Hosts -> Row(Modifier.fillMaxSize()) {
                                        if (twoPanes || !hostDetailVisible) {
                                        hostStates.SaveableStateProvider("host-list") {
                                        HostsScreen(
                                            modifier = Modifier.weight(1f),
                                            hosts = hosts,
                                            recentHosts = recentHosts,
                                            expandedHostId = if (twoPanes) null else expandedHostId,
                                            onExpandedChange = { expandedHostId = it; hostDetailVisible = twoPanes && it != null },
                                            onToggleFavorite = { host ->
                                                scope.launch { repository.toggleFavorite(host.id) }
                                            },
                                            onAddHost = {
                                                editorForNewHost = true
                                            },
                                            onConnectHost = { host -> openConnection(host, sftp = false) },
                                            onEditHost = { host ->
                                                editorProfileId = host.id
                                            },
                                            onAgentHost = if (agentChat != null) { { host -> agentChatVisible = false; agentSheetHostId = host.id } } else null,
                                            onSftpHost = { host -> openConnection(host, sftp = true) }
                                        )
                                        }
                                        }
                                        if (twoPanes || hostDetailVisible) {
                                            if (twoPanes) VerticalDivider()
                                            Box(Modifier.weight(1f).fillMaxHeight().padding(UiSpacing.sm)) {
                                                hostStates.SaveableStateProvider("host-detail") {
                                                val detail = hosts.firstOrNull { it.id == expandedHostId }
                                                if (detail == null) Text(stringResource(R.string.ux_choose_host))
                                                else androidx.compose.foundation.layout.Column(Modifier.verticalScroll(rememberScrollState())) {
                                                    Text(stringResource(R.string.ux_host_detail), style = MaterialTheme.typography.headlineSmall)
                                                    HostRow(detail, expanded = true, onExpandedChange = { expandedHostId = null; hostDetailVisible = false },
                                                        onConnect = { openConnection(detail, false) },
                                                        onSftp = { openConnection(detail, true) },
                                                        onEdit = { editorProfileId = detail.id },
                                                        onToggleFavorite = { scope.launch { repository.toggleFavorite(detail.id) } },
                                                        onAgent = if (agentChat != null) { { agentSheetHostId = detail.id } } else null)
                                                }
                                                }
                                            }
                                        }
                                        }

                                        MainDestination.Tools -> ToolsScreen(
                                            onToolClick = {
                                                toolSearchQuery = ""
                                                selectedToolName = it.name
                                            }
                                        )

                                        MainDestination.Settings -> AnimatedDetailPane(
                                            // Compact details live above the stable main scaffold.
                                            selected = selectedSetting.takeIf { twoPanes },
                                            twoPanes = twoPanes,
                                            defaultDetail = SettingsDestination.Appearance,
                                            listContent = {
                                                detailStates.SaveableStateProvider("settings-list") {
                                                    SettingsScreen(
                                                        modifier = Modifier.fillMaxSize().testTag("settings-list"),
                                                        onSettingClick = {
                                                            settingFocusAnchor = null
                                                            selectedSettingName = it.name
                                                        }
                                                    )
                                                }
                                            },
                                            detailContent = settingDetailContent
                                        )
                                    }
                                    }
                                }
                            }
                        }
                    }
                    // Keep chrome and outgoing tab bounds fixed during tab changes. Compact
                    // details own a full-screen surface; resizing removes it before the wide
                    // pane registers the same saved detail slot.
                    if (!twoPanes) {
                        SessionSurface(
                            state = selectedSetting.takeIf { selected == MainDestination.Settings },
                            content = settingDetailContent
                        )
                    }
                    }
                }
            }
        }

        // Capture presentation data in the transition target: clearing the live
        // connection fields must not remove the outgoing content before its fade.
        connectionTransition.AnimatedContent(
            contentKey = { it?.profile?.id },
            transitionSpec = { MiaominalMotion.surfaceTransition() },
            modifier = Modifier.fillMaxSize()
        ) { presentation ->
            presentation?.let {
                ConnectingScreen(
                    profile = it.profile,
                    sftp = it.sftp,
                    stage = it.stage,
                    phase = it.phase,
                    failure = it.failure,
                    requestKind = it.requestKind,
                    active = connecting,
                    onCancel = { cancelConnection() },
                    onRetry = { cancelConnection(); openConnection(it.profile, it.sftp) },
                    onEditProfile = { cancelConnection(); editorProfileId = it.profile.id }
                )
            }
        }

        // The SFTP browser stays above the base layer so a terminal session survives the file manager.
        SessionSurface(state = sftpProfile, contentKey = { it?.id }) { shownProfile ->
            var sftpDrawerOpen by rememberSaveable { mutableStateOf(false) }
            SessionDrawer(
                open = sftpDrawerOpen,
                onClose = { sftpDrawerOpen = false },
                drawerContent = {
                    OpenedLinksDrawer(
                        links = sshLinks + sftpLinks + agentLinks,
                        selectedLinkId = sftpShownConnectionId?.let { "sftp:$it" },
                        onClose = { sftpDrawerOpen = false },
                        onSelect = { link ->
                            sftpDrawerOpen = false
                            when (link.kind) {
                                OpenedLinkKind.Agent -> { agentChatVisible = true }
                                OpenedLinkKind.Ssh -> {
                                    // Switch to the terminal; the SFTP session
                                    // stays open and keeps its drawer entry.
                                    sftpProfileId = null
                                    sftpAttachConnectionId = null
                                    sftpShownConnectionId = null
                                    sftpOriginTerminalId = null
                                    activeConnectionId = link.connectionId
                                }

                                OpenedLinkKind.Sftp -> {
                                    sftpSessions.firstOrNull { it.connectionId == link.connectionId }
                                        ?.let { sftp ->
                                            sftpAttachConnectionId = sftp.connectionId
                                            sftpShownConnectionId = sftp.connectionId
                                            sftpProfileId = sftp.hostId
                                        }
                                }
                            }
                        },
                        onCloseLink = { link ->
                            sftpDrawerOpen = false
                            when (link.kind) {
                                OpenedLinkKind.Agent -> run { confirmEndAgent = true }
                                OpenedLinkKind.Ssh -> closeSession(link.connectionId)
                                OpenedLinkKind.Sftp -> {
                                    requestSftpClose(link.connectionId)
                                }
                            }
                        }
                    )
                },
                modifier = Modifier.fillMaxSize()
            ) {
                sessionStates.SaveableStateProvider("sftp:${shownProfile.id}") {
                SftpScreen(
                    profile = shownProfile,
                    attachConnectionId = sftpAttachConnectionId,
                    onMenu = { sftpDrawerOpen = true },
                    onSessionChanged = { sftpShownConnectionId = it },
                    onRequestClose = ::requestSftpClose,
                    onEditProfile = { sftpProfileId = null; editorProfileId = shownProfile.id },
                    onDismiss = {
                        sftpProfileId = null
                        sftpAttachConnectionId = null
                        sftpShownConnectionId = null
                        sftpOriginTerminalId?.toLongOrNull()?.let { origin ->
                            if (liveSessions.any { it.connectionId == origin }) activeConnectionId = origin
                        }
                        sftpOriginTerminalId = null
                    },
                    modifier = Modifier.fillMaxSize()
                )
                }
            }
            // Back closes the drawer before it leaves the SFTP screen.
            BackHandler(enabled = sftpDrawerOpen) {
                sftpDrawerOpen = false
            }
        }

        // AI is entered from Hosts or the terminal, and stays alive across navigation.
        val sheetHostId = agentSheetHostId
        SessionSurface(state = sheetHostId?.takeIf { agentChat != null }) { launchHostId ->
            val profile = hosts.firstOrNull { it.id == launchHostId }
            val snapshot = liveSessions.firstOrNull { it.hostId == launchHostId }
            AgentLaunchSheet(
                agentChat = requireNotNull(agentChat),
                hostId = launchHostId,
                hostLabel = profile?.name ?: snapshot?.hostLabel.orEmpty(),
                shellType = profile?.shellType ?: dev.akko.miaominal.data.ShellType.Posix,
                onDismiss = { agentSheetHostId = null },
                onOpened = {
                    agentSheetHostId = null
                    agentChatVisible = true
                }
            )
        }

        SessionSurface(state = agentChat?.takeIf { agentChatVisible && agentState.session != null }) { chat ->
            val agentHostLabel = hosts.firstOrNull { it.id == agentState.session?.hostId }?.name
                ?: agentState.session?.hostId.orEmpty()
            var agentDrawerOpen by rememberSaveable { mutableStateOf(false) }
            SessionDrawer(
                open = agentDrawerOpen,
                onClose = { agentDrawerOpen = false },
                drawerContent = {
                    OpenedLinksDrawer(
                        links = sshLinks + sftpLinks + agentLinks,
                        selectedLinkId = "agent:current",
                        onClose = { agentDrawerOpen = false },
                        onSelect = { link ->
                            agentDrawerOpen = false
                            when (link.kind) {
                                OpenedLinkKind.Agent -> Unit
                                OpenedLinkKind.Ssh -> {
                                    agentChatVisible = false
                                    agentSheetHostId = null
                                    sftpProfileId = null
                                    activeConnectionId = link.connectionId
                                }
                                OpenedLinkKind.Sftp -> sftpSessions.firstOrNull {
                                    it.connectionId == link.connectionId
                                }?.let {
                                    agentChatVisible = false
                                    agentSheetHostId = null
                                    sftpAttachConnectionId = it.connectionId
                                    sftpShownConnectionId = it.connectionId
                                    sftpProfileId = it.hostId
                                }
                            }
                        },
                        onCloseLink = { link ->
                            when (link.kind) {
                                OpenedLinkKind.Agent -> {
                                    confirmEndAgent = true
                                }
                                OpenedLinkKind.Ssh -> closeSession(link.connectionId)
                                OpenedLinkKind.Sftp -> requestSftpClose(link.connectionId)
                            }
                        }
                    )
                },
                modifier = Modifier.fillMaxSize()
            ) {
                AgentChatScreen(
                    files = agentFiles,
                    onAttachFile = { agentFilePickerOpen = true },
                    onRemoveFile = { chat.removeFile(it) },
                    onSelectReasoning = { id -> scope.launch { chat.selectReasoning(id) } },
                    onMenu = { agentDrawerOpen = true },
                    onLeaveChat = { agentChatVisible = false },
                    onReconnect = { chat.reconnectSession() },
                    state = agentState,
                    hostLabel = agentHostLabel,
                    draft = agentDraft,
                    onDraftChange = { chat.setDraft(it) },
                    onSend = {
                        val text = agentDraft.trim()
                        if (text.isNotEmpty() || agentFiles.isNotEmpty()) {
                            scope.launch { chat.sendMessage(text) }
                        }
                    },
                    onStop = { scope.launch { chat.stopTurn() } },
                    onRespondOption = { requestId, optionId ->
                        scope.launch {
                            chat.respondInteraction(
                                requestId,
                                AgentInteractionAnswer.Option(optionId)
                            )
                        }
                    },
                    onRespondAnswers = { requestId, answers ->
                        scope.launch {
                            chat.respondInteraction(
                                requestId,
                                AgentInteractionAnswer.Answers(
                                    answers.map { (questionId, values) ->
                                        InteractionAnswerEntry(questionId, values)
                                    }
                                )
                            )
                        }
                    },
                    onSelectModel = { model -> chat.selectModel(model) },
                    onSwitchSession = {
                        // New conversation or switch to a history entry; this is an
                        // explicit action because it closes the running session.
                        val chatHostId = agentState.session?.hostId
                            ?: activeConnectionId?.let { profileForConnection(it)?.id }
                        if (chatHostId != null) {
                            agentChatVisible = false
                            agentSheetHostId = chatHostId
                        }
                    },
                    onCloseSession = {
                        confirmEndAgent = true
                    },
                    modifier = Modifier.fillMaxSize()
                )
            }
            if (agentFilePickerOpen) agentState.session?.let { session ->
                val shell = hosts.firstOrNull { it.id == session.hostId }?.shellType
                    ?: dev.akko.miaominal.data.ShellType.Posix
                dev.akko.miaominal.ui.screens.RemoteDirectoryPicker(
                    hostId = session.hostId,
                    initialPath = dev.akko.miaominal.data.agent.AgentCommands.toSftpPath(session.cwd, shell),
                    selectFile = true,
                    onDismiss = { agentFilePickerOpen = false },
                    onSelect = {
                        chat.addFile(dev.akko.miaominal.data.agent.AgentCommands.fromSftpPath(it, shell))
                        agentFilePickerOpen = false
                    }
                )
            }
            BackHandler {
                if (agentDrawerOpen) agentDrawerOpen = false
                else {
                    agentChatVisible = false
                    agentSheetHostId = agentState.session?.hostId
                }
            }
        }

        navigation.pendingClose?.let { encoded ->
            val id = encoded.substringAfter(':').toLongOrNull()
            val sftp = encoded.startsWith("sftp:")
            val count = id?.let { repository.sftpSessionHandle(it)?.transfersSnapshot()?.count { task -> !task.state.isTerminal } } ?: 0
            androidx.compose.material3.AlertDialog(onDismissRequest = { navigation.pendingClose = null },
                title = { Text(stringResource(R.string.ux_close_title)) },
                text = { Text(if (sftp) stringResource(R.string.ux_close_sftp, count) else stringResource(R.string.ux_close_ssh)) },
                confirmButton = { androidx.compose.material3.TextButton(onClick = {
                    navigation.pendingClose = null
                    if (id != null) { if (sftp) performSftpClose(id) else performCloseSession(id) }
                }) { Text(stringResource(R.string.ux_end_session)) } },
                dismissButton = { androidx.compose.material3.TextButton(onClick = { navigation.pendingClose = null }) { Text(stringResource(R.string.ux_keep_session)) } })
        }

        SnackbarHost(snackbar, modifier = Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(bottom = 88.dp))

        if (confirmEndAgent) androidx.compose.material3.AlertDialog(
            onDismissRequest = { confirmEndAgent = false },
            title = { Text(stringResource(R.string.agent_end_title)) },
            text = { Text(stringResource(R.string.agent_end_detail)) },
            confirmButton = {
                androidx.compose.material3.TextButton(onClick = {
                    confirmEndAgent = false
                    agentChatVisible = false
                    agentSheetHostId = null
                    agentFilePickerOpen = false
                    scope.launch { agentChat?.closeSession() }
                }) { Text(stringResource(R.string.ui_close_session)) }
            },
            dismissButton = {
                androidx.compose.material3.TextButton(onClick = { confirmEndAgent = false }) {
                    Text(stringResource(R.string.agent_keep_current))
                }
            }
        )

        // Trust/authentication prompts are derived from backend state, so an
        // Activity rebuild shows the same pending request.
        pendingRequest?.let { request ->
            val handle = repository.sessionHandle(request.connectionId)
            SessionRequestDialog(
                request = request,
                onHostKey = { requestId, decision ->
                    handle?.respondHostKey(requestId, decision)
                    scope.launch { repository.refreshSessions() }
                },
                onKeyboardInteractive = { requestId, answers ->
                    handle?.respondKeyboardInteractive(requestId, answers)
                    scope.launch { repository.refreshSessions() }
                },
                onCancel = {
                    if (handle != null) {
                        handle.cancel()
                    } else {
                        closeSession(request.connectionId)
                    }
                }
            )
        }

        // Forwarding trust/authentication prompts are global: the session lives
        // in the backend registry, so the answer does not depend on which page
        // is visible.
        forwardSessions.firstOrNull { it.request != null }?.let { session ->
            val request = session.request
            if (request != null) {
                val handle = repository.forwardSessionHandle(session.profileId, session.ruleId)
                SessionRequestDialog(
                    request = request.toSessionRequest(),
                    onHostKey = { requestId, decision ->
                        handle?.answerHostKey(requestId, decision)
                        scope.launch { repository.refreshForwardSessions() }
                    },
                    onKeyboardInteractive = { requestId, answers ->
                        handle?.answerKeyboardInteractive(requestId, answers)
                        scope.launch { repository.refreshForwardSessions() }
                    },
                    onCancel = {
                        handle?.cancel()
                        scope.launch { repository.refreshForwardSessions() }
                    }
                )
            }
        }

        if (!preferences.onboardingCompleted) {
            OnboardingScreen(
                onAddHost = { editorForNewHost = true },
                onImportHosts = { selectedName = MainDestination.Settings.name; selectedSettingName = SettingsDestination.Connections.name; settingFocusAnchor = "connections-import" },
                onComplete = {
                    scope.launch {
                        repository.updateSettings { it.copy(onboardingCompleted = true) }
                    }
                },
                modifier = Modifier.fillMaxSize()
            )
        }
    }
}

/** Owns the status/navigation bar icon color for the whole app so screens never capture-and-restore it. */
@Composable
private fun SystemBarAppearance(light: Boolean) {
    val view = LocalView.current
    if (view.isInEditMode) return
    SideEffect {
        val window = (view.context as? Activity)?.window ?: return@SideEffect
        val controller = WindowCompat.getInsetsController(window, view)
        controller.isAppearanceLightStatusBars = light
        controller.isAppearanceLightNavigationBars = light
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MiaominalNavigationBar(
    selected: MainDestination,
    onSelected: (MainDestination) -> Unit
) {
    NavigationBar(
        modifier = Modifier.testTag("main-navigation-bar"),
        containerColor = MaterialTheme.colorScheme.surfaceContainerLowest,
        tonalElevation = 0.dp
    ) {
        MainDestination.entries.forEach { destination ->
            val isSelected = destination == selected
            NavigationBarItem(
                selected = isSelected,
                onClick = { onSelected(destination) },
                icon = {
                    Icon(
                        imageVector = if (isSelected) destination.selectedIcon else destination.unselectedIcon,
                        contentDescription = null
                    )
                },
                label = { Text(stringResource(destination.labelRes)) }
            )
        }
    }
}

@Composable
private fun MiaominalNavigationRail(
    selected: MainDestination,
    onSelected: (MainDestination) -> Unit
) {
    NavigationRail(containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        Spacer(Modifier.size(12.dp))
        MainDestination.entries.forEach { destination ->
            val isSelected = destination == selected
            NavigationRailItem(
                selected = isSelected,
                onClick = { onSelected(destination) },
                icon = {
                    Icon(
                        imageVector = if (isSelected) destination.selectedIcon else destination.unselectedIcon,
                        contentDescription = null
                    )
                },
                label = { Text(stringResource(destination.labelRes)) }
            )
        }
    }
}
