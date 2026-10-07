package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.ui.components.PageEmptyState
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.uiString

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import dev.akko.miaominal.ui.components.SessionTopBarCapsule
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.material3.FloatingActionButton
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.automirrored.outlined.ArrowForward
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.History
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusManager
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.data.ShellType
import dev.akko.miaominal.data.agent.AgentChatRepository
import dev.akko.miaominal.data.agent.AgentCli
import dev.akko.miaominal.data.agent.AgentCommands
import dev.akko.miaominal.data.agent.AgentHistoryEntry
import dev.akko.miaominal.data.agent.AgentLaunchResult

import dev.akko.miaominal.data.agent.historyWorkspaces
import kotlinx.coroutines.launch

const val AgentLaunchSheetTag = "agent-launch-sheet"
const val AgentNewConversationTag = "agent-new-conversation"
const val AgentNewConversationOpenTag = "agent-new-conversation-open"
const val AgentPrepareTag = "agent-prepare"
const val AgentCwdFieldTag = "agent-cwd-field"
const val AgentCwdPickerButtonTag = "agent-cwd-picker-button"

/**
 * Launch sheet: history is the main content. Opening the panel connects and
 * loads every conversation the CLI knows about; the workspace field is only
 * needed to start a new conversation and defaults to the newest workspace.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AgentLaunchSheet(
    agentChat: AgentChatRepository,
    hostId: String,
    hostLabel: String,
    shellType: ShellType = ShellType.Posix,
    onDismiss: () -> Unit,
    onOpened: () -> Unit,
    modifier: Modifier = Modifier
) {
    val scope = rememberCoroutineScope()
    val configRepository = dev.akko.miaominal.ui.LocalAgentConfigRepository.current
    val configState = configRepository?.state?.collectAsState()?.value
    val sessionState by agentChat.state.collectAsState()
    val initialSession = agentChat.currentSession()?.takeIf { it.hostId == hostId }
    var cliName by rememberSaveable(hostId) { mutableStateOf(initialSession?.cli?.name ?: AgentCli.Codex.name) }
    var cwd by rememberSaveable(hostId) { mutableStateOf(initialSession?.cwd.orEmpty()) }
    var preparing by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<AgentLaunchResult?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var failedHistoryEntry by remember { mutableStateOf<AgentHistoryEntry?>(null) }
    var newConversationError by remember { mutableStateOf<String?>(null) }
    var directoryPickerOpen by rememberSaveable { mutableStateOf(false) }
    var confirmReplace by remember { mutableStateOf(false) }
    var opening by remember { mutableStateOf(false) }
    var newConversationOpen by rememberSaveable { mutableStateOf(false) }
    val cli = AgentCli.valueOf(cliName)
    var configSource by remember(hostId, cliName) { mutableStateOf<dev.akko.miaominal.data.agent.AgentConfigSource>(dev.akko.miaominal.data.agent.AgentConfigSource.Remote) }
    var selectionLoaded by remember(hostId, cliName) { mutableStateOf(configRepository == null) }
    var selectionError by remember(hostId, cliName) { mutableStateOf<String?>(null) }
    var configManagerOpen by remember { mutableStateOf(false) }
    var pendingConfigSource by remember { mutableStateOf<dev.akko.miaominal.data.agent.AgentConfigSource?>(null) }
    val ready = result as? AgentLaunchResult.Ready
    val launchResult = result
    val focusManager: FocusManager = LocalFocusManager.current
    val keyboardController = LocalSoftwareKeyboardController.current

    fun prepare() {
        if (preparing || !selectionLoaded || selectionError != null) return
        focusManager.clearFocus()
        keyboardController?.hide()
        preparing = true
        error = null
        failedHistoryEntry = null
        result = null
        scope.launch {
            try {
                val prepared = agentChat.prepare(hostId, cli, cwd.trim(), configSource)
                result = prepared
                // History is loaded from every workspace; adopt the newest one
                // as the default so a new conversation needs no typed path.
                if (prepared is AgentLaunchResult.Ready && cwd.isBlank()) {
                    cwd = historyWorkspaces(prepared.history).firstOrNull().orEmpty()
                }
            } finally {
                preparing = false
            }
        }
    }

    // Opening the panel connects on its own and loads history immediately.
    // A live conversation is never replaced here: switching away from one
    // stays an explicit confirmation, so a running turn cannot be interrupted
    // silently. An idle prepared process (no conversation yet) is fair game.
    val keepLiveConversation =
        agentChat.isActive && sessionState.session?.remoteSessionId != null
    LaunchedEffect(hostId, cliName, configRepository) {
        if (configRepository != null) {
            configRepository.selection(hostId, cli)
                .onSuccess { remembered ->
                    configSource = if (initialSession?.cli == cli && keepLiveConversation)
                        agentChat.currentConfigSource() else remembered
                    selectionError = null
                    selectionLoaded = true
                }
                .onFailure { selectionError = it.message; selectionLoaded = true }
        }
    }
    LaunchedEffect(hostId, cliName, selectionLoaded, configSource) {
        if (selectionLoaded && selectionError == null && !keepLiveConversation && result == null && !preparing) {
            prepare()
        }
    }

    fun changeConfiguration(source: dev.akko.miaominal.data.agent.AgentConfigSource, replace: Boolean = false) {
        if (keepLiveConversation && !replace) {
            pendingConfigSource = source
            confirmReplace = true
            return
        }
        scope.launch {
            val remembered = configRepository?.remember(hostId, cli, source)
            if (remembered?.isFailure == true && source != dev.akko.miaominal.data.agent.AgentConfigSource.Remote) {
                selectionError = remembered.exceptionOrNull()?.message
                return@launch
            }
            configSource = source
            selectionError = null
            result = null
            error = null
            failedHistoryEntry = null
            newConversationError = null
            if (replace || !keepLiveConversation) prepare()
        }
    }

    fun openSession(block: suspend () -> Result<Unit>, onFailure: (String) -> Unit) {
        if (opening) return
        opening = true
        scope.launch {
            try {
                block()
                    .onSuccess { onOpened() }
                    .onFailure { onFailure(it.message ?: it.javaClass.simpleName) }
            } finally {
                opening = false
            }
        }
    }

    Surface(modifier = modifier.fillMaxSize().testTag(AgentLaunchSheetTag),
        color = MaterialTheme.colorScheme.surface) {
        Box(Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .statusBarsPadding().navigationBarsPadding()
                .imePadding()
                .padding(horizontal = 20.dp)
                .padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            SessionTopBarCapsule {
                IconButton(onClick = onDismiss) {
                    Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.agent_close_detail))
                }
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(2.dp)
                ) {
                    Text(
                        text = uiString(R.string.ui_ai_session),
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onSurface
                    )
                    Text(
                        text = hostLabel.ifBlank { hostId },
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                }
            }

            SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                AgentCli.entries.forEachIndexed { index, entry ->
                    SegmentedButton(
                        enabled = !preparing && !opening,
                        selected = cli == entry,
                        onClick = {
                            cliName = entry.name
                            result = null
                            error = null
                            failedHistoryEntry = null
                            newConversationError = null
                        },
                        shape = SegmentedButtonDefaults.itemShape(
                            index = index,
                            count = AgentCli.entries.size
                        )
                    ) {
                        Text(entry.displayName)
                    }
                }
            }

            if (configRepository != null) {
                AgentConfigSelector(
                    profiles = configState?.profiles.orEmpty().filter { it.supports(cli) },
                    source = configSource,
                    enabled = selectionLoaded && !preparing && !opening,
                    onSelect = { if (it != configSource || selectionError != null) changeConfiguration(it) },
                    onManage = { configManagerOpen = true }
                )
            }

            when {
                selectionError != null -> LaunchErrorCard(
                    modifier = Modifier.weight(1f), fullPage = true,
                    title = stringResource(R.string.agent_config_load_failed),
                    detail = selectionError.orEmpty(),
                    onRetry = { scope.launch {
                        configRepository?.selection(hostId, cli)?.onSuccess {
                            configSource = it; selectionError = null; prepare()
                        }?.onFailure { selectionError = it.message }
                    } }
                )
                !selectionLoaded -> CircularProgressIndicator()
                preparing && result == null -> Column(
                    modifier = Modifier.fillMaxWidth().weight(1f),
                    verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    CircularProgressIndicator(modifier = Modifier.size(40.dp))
                    Text(
                        text = uiString(R.string.ui_starting),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }

                opening && failedHistoryEntry != null -> Column(
                    modifier = Modifier.fillMaxWidth().weight(1f),
                    verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    CircularProgressIndicator(modifier = Modifier.size(40.dp))
                    Text(stringResource(R.string.agent_launch_opening_history),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                }

                result == null -> Button(
                    onClick = {
                        if (agentChat.isActive) confirmReplace = true else prepare()
                    },
                    enabled = !preparing,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag(AgentPrepareTag)
                ) {
                    Text(uiString(R.string.ui_connect_and_load_history))
                }

                error != null && failedHistoryEntry != null -> LaunchErrorCard(
                    modifier = Modifier.weight(1f),
                    fullPage = true,
                    title = uiString(R.string.ui_could_not_open_session),
                    detail = error.orEmpty(),
                    onRetry = {
                        failedHistoryEntry?.let { entry ->
                            error = null
                            openSession(
                                block = { agentChat.resumeSession(entry.remoteId, entry.cwd) },
                                onFailure = { error = it }
                            )
                        }
                    },
                    onDismiss = {
                        error = null
                        failedHistoryEntry = null
                    }
                )

                ready != null -> {
                    ready.historyError?.let { detail ->
                        LaunchErrorCard(
                            title = stringResource(R.string.agent_history_failed),
                            detail = detail,
                            onRetry = if (opening) null else ::prepare
                        )
                    }
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                    HistorySection(
                        modifier = Modifier.weight(1f),
                        showEmpty = ready.historyError == null,
                        currentId = sessionState.session?.remoteSessionId,
                        enabled = !opening,
                        entries = ready.history,
                        onSelect = { entry ->
                            failedHistoryEntry = entry
                            error = null
                            openSession(
                                block = { agentChat.resumeSession(entry.remoteId, entry.cwd) },
                                onFailure = { error = it }
                            )
                        }
                    )
                }

                launchResult is AgentLaunchResult.Unavailable -> LaunchErrorCard(
                    modifier = Modifier.weight(1f),
                    fullPage = true,
                    title = uiString(R.string.ui_is_unavailable, cli.displayName),
                    detail = launchResult.detail,
                    onRetry = ::prepare
                )

                launchResult is AgentLaunchResult.Failed -> LaunchErrorCard(
                    modifier = Modifier.weight(1f),
                    fullPage = true,
                    title = uiString(R.string.ui_could_not_start, cli.displayName),
                    detail = launchResult.detail,
                    onRetry = ::prepare
                )
            }

        }
            if (ready != null && !opening && error == null) FloatingActionButton(
                onClick = { newConversationError = null; newConversationOpen = true },
                modifier = Modifier.align(Alignment.BottomEnd)
                    .navigationBarsPadding().imePadding().padding(20.dp)
                    .testTag(AgentNewConversationOpenTag)
            ) {
                Icon(Icons.Outlined.Add, stringResource(R.string.agent_launch_new))
            }
        }
    }

    if (newConversationOpen) ModalBottomSheet(
        onDismissRequest = { if (!opening) newConversationOpen = false },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ) {
        Column(Modifier.fillMaxWidth().imePadding().verticalScroll(rememberScrollState()).padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.agent_launch_new), style = MaterialTheme.typography.titleLarge)
            Surface(shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.surfaceContainerLow) {
                Column(
                modifier = Modifier.fillMaxWidth().padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                OutlinedTextField(
                    enabled = !preparing && !opening,
                    value = cwd,
                    onValueChange = {
                        // The workspace only affects new conversations; the loaded
                        // history stays visible while the field is edited.
                        cwd = it
                    },
                    label = { Text(uiString(R.string.ui_working_directory)) },
                    placeholder = {
                        Text(
                            text = uiString(R.string.ui_remote_path_hint),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                    },
                    trailingIcon = {
                        IconButton(
                            enabled = !preparing && !opening,
                            onClick = {
                                focusManager.clearFocus()
                                keyboardController?.hide()
                                directoryPickerOpen = true
                            },
                            modifier = Modifier.testTag(AgentCwdPickerButtonTag)
                        ) {
                            Icon(
                                Icons.Outlined.FolderOpen,
                                contentDescription = stringResource(R.string.agent_browse_remote_directory)
                            )
                        }
                    },
                    singleLine = true,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag(AgentCwdFieldTag)
                )
                val workspaces = ready?.history?.let(::historyWorkspaces).orEmpty().filter { it != cwd.trim() }
                if (workspaces.isNotEmpty()) {
                WorkspaceChips(
                    workspaces = workspaces,
                    enabled = !opening,
                    onSelect = { path -> cwd = path }
                )
            }
                Button(
                    enabled = ready != null && cwd.isNotBlank() && !opening,
                    onClick = {
                        newConversationError = null
                        openSession(block = {
                            // The directory is chosen after history loading; prepare the CLI
                            // in that workspace before creating the remote conversation.
                            if (agentChat.currentSession()?.cwd != cwd.trim()) {
                                val prepared = agentChat.prepare(hostId, cli, cwd.trim(), configSource)
                                result = prepared
                                if (prepared !is AgentLaunchResult.Ready) {
                                    val detail = when (prepared) {
                                        is AgentLaunchResult.Failed -> prepared.detail
                                        is AgentLaunchResult.Unavailable -> prepared.detail
                                        else -> ""
                                    }
                                    return@openSession Result.failure(IllegalStateException(detail))
                                }
                            }
                            agentChat.startNewSession()
                        }, onFailure = { newConversationError = it })
                    },
                    modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(AgentNewConversationTag)
                ) {
                    Icon(Icons.Outlined.Add, null, Modifier.size(20.dp))
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.agent_launch_new))
                }

            }
            }

            newConversationError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            if (opening) CircularProgressIndicator(Modifier.size(24.dp))
        }
    }

    if (confirmReplace) AlertDialog(
        onDismissRequest = { confirmReplace = false; pendingConfigSource = null },
        title = { Text(stringResource(R.string.agent_replace_title)) },
        text = { Text(stringResource(R.string.agent_replace_detail)) },
        confirmButton = { TextButton(onClick = {
            confirmReplace = false
            val pending = pendingConfigSource
            pendingConfigSource = null
            if (pending != null) changeConfiguration(pending, replace = true) else prepare()
        }) {
            Text(stringResource(R.string.agent_replace_confirm))
        } },
        dismissButton = { TextButton(onClick = { confirmReplace = false; pendingConfigSource = null }) {
            Text(stringResource(R.string.agent_keep_current))
        } }
    )

    if (configManagerOpen) ModalBottomSheet(
        onDismissRequest = { configManagerOpen = false },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ) {
        Column(Modifier.fillMaxWidth().fillMaxHeight(0.9f)) {
            Text(stringResource(R.string.agent_config_title), style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(horizontal = 20.dp))
            AgentConfigSettingsPage(Modifier.weight(1f), configRepository)
        }
    }

    if (directoryPickerOpen) {
        RemoteDirectoryPicker(
            hostId = hostId,
            initialPath = AgentCommands.toSftpPath(cwd, shellType),
            onDismiss = { directoryPickerOpen = false },
            onSelect = { selected ->
                cwd = AgentCommands.fromSftpPath(selected, shellType)
                error = null
                directoryPickerOpen = false
            }
        )
    }
}

/** One scrolling row of known workspaces; no separate section header. */
@Composable
private fun WorkspaceChips(
    workspaces: List<String>,
    enabled: Boolean,
    onSelect: (String) -> Unit
) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(
            text = uiString(R.string.ui_existing_workspaces),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Spacer(Modifier.width(8.dp))
        androidx.compose.foundation.lazy.LazyRow(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            items(workspaces.size, key = { workspaces[it] }) { index ->
                val path = workspaces[index]
                AssistChip(
                    enabled = enabled,
                    onClick = { onSelect(path) },
                    leadingIcon = {
                        Icon(
                            Icons.Outlined.FolderOpen,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp)
                        )
                    },
                    label = { Text(path, maxLines = 1, overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.widthIn(max = 240.dp)) }
                )
            }
        }
    }
}

@Composable
private fun HistorySection(
    entries: List<AgentHistoryEntry>,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    showEmpty: Boolean = true,
    currentId: String? = null,
    onSelect: (AgentHistoryEntry) -> Unit
) {
    var query by rememberSaveable { mutableStateOf("") }
    val filtered = entries.filter {
        it.title.contains(query, ignoreCase = true) ||
            it.preview.orEmpty().contains(query, ignoreCase = true) ||
            it.cwd.orEmpty().contains(query, ignoreCase = true)
    }
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                Icons.Outlined.History,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(18.dp)
            )
            Text(
                text = uiString(R.string.ui_existing_conversations),
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurface
            )
            Spacer(Modifier.weight(1f))
            if (entries.isNotEmpty()) {
                Text(
                    text = "${entries.size}",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
        if (entries.isNotEmpty()) {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                placeholder = {
                    Text(
                        text = stringResource(R.string.agent_search_conversations),
                        maxLines = 1
                    )
                },
                leadingIcon = {
                    Icon(Icons.Outlined.Search, contentDescription = null, modifier = Modifier.size(18.dp))
                },
                singleLine = true,
                shape = MaterialTheme.shapes.extraLarge,
                modifier = Modifier.fillMaxWidth()
            )
        }
        when {
            entries.isEmpty() && showEmpty -> PageEmptyState(
                icon = Icons.Outlined.History,
                title = uiString(R.string.ui_no_conversations_in_this_directory),
                modifier = Modifier.weight(1f)
            )
            entries.isNotEmpty() && filtered.isEmpty() -> PageEmptyState(
                icon = Icons.Outlined.Search,
                title = stringResource(R.string.agent_no_matches),
                text = stringResource(R.string.page_search_empty_hint),
                modifier = Modifier.weight(1f)
            )

            entries.isNotEmpty() -> androidx.compose.foundation.lazy.LazyColumn(
                contentPadding = PaddingValues(bottom = 88.dp),
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f)
            ) {
                val sorted = filtered.sortedByDescending { it.updatedAtMs ?: 0L }
                items(sorted.size, key = { sorted[it].remoteId }) { index ->
                    val entry = sorted[index]
                    val day = entry.updatedAtMs?.let {
                        java.text.DateFormat.getDateInstance(java.text.DateFormat.MEDIUM).format(java.util.Date(it))
                    }.orEmpty()
                    val previousDay = sorted.getOrNull(index - 1)?.updatedAtMs?.let {
                        java.text.DateFormat.getDateInstance(java.text.DateFormat.MEDIUM).format(java.util.Date(it))
                    }
                    if (index == 0 || day != previousDay) {
                        Text(
                            text = day.ifBlank { stringResource(R.string.agent_earlier) },
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(top = 6.dp, bottom = 2.dp)
                        )
                    }
                    HistoryRow(
                        entry = entry,
                        current = entry.remoteId == currentId,
                        enabled = enabled,
                        onClick = { onSelect(entry) }
                    )
                }
            }
        }
    }
}

@Composable
private fun HistoryRow(
    entry: AgentHistoryEntry,
    current: Boolean,
    enabled: Boolean,
    onClick: () -> Unit
) {
    val workspace = entry.cwd?.takeIf(String::isNotBlank)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(1.dp)
        ) {
            Text(
                text = entry.title,
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
            workspace?.let {
                Text(
                    text = it,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
            }
            entry.preview?.takeIf { it.isNotBlank() && it != workspace }?.let {
                Text(
                    text = it,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
            }
        }
        if (current) {
            Spacer(Modifier.width(8.dp))
            Text(
                text = stringResource(R.string.agent_current_selection),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.primary
            )
        } else {
            Icon(Icons.AutoMirrored.Outlined.ArrowForward, null,
                modifier = Modifier.padding(start = 12.dp).size(18.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun LaunchErrorCard(
    title: String,
    detail: String,
    onRetry: (() -> Unit)?,
    modifier: Modifier = Modifier,
    fullPage: Boolean = false,
    onDismiss: (() -> Unit)? = null
) {
    var detailsOpen by rememberSaveable(detail) { mutableStateOf(false) }
    if (fullPage) Box(modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Column(
            modifier = Modifier.widthIn(max = 360.dp).verticalScroll(rememberScrollState()).padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            Icon(Icons.Outlined.ErrorOutline, null,
                tint = MaterialTheme.colorScheme.error, modifier = Modifier.size(48.dp))
            Text(title, style = MaterialTheme.typography.titleLarge,
                color = MaterialTheme.colorScheme.onSurface, textAlign = TextAlign.Center)
            Text(stringResource(R.string.agent_launch_error_hint),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
            onRetry?.let { retry ->
                Button(onClick = retry) { Text(uiString(R.string.sftp_retry)) }
            }
            onDismiss?.let { dismiss ->
                TextButton(onClick = dismiss) {
                    Text(stringResource(R.string.agent_launch_back_to_history))
                }
            }
            if (detail.isNotBlank()) TextButton(onClick = { detailsOpen = true }) {
                Text(stringResource(R.string.agent_launch_error_details))
            }
        }
    } else Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        modifier = modifier.fillMaxWidth()
    ) {
        Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Surface(shape = MaterialTheme.shapes.medium,
                    color = MaterialTheme.colorScheme.errorContainer) {
                    Icon(Icons.Outlined.ErrorOutline, null,
                        tint = MaterialTheme.colorScheme.onErrorContainer,
                        modifier = Modifier.padding(12.dp).size(24.dp))
                }
                Text(title, style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurface, modifier = Modifier.weight(1f))
            }
            Text(stringResource(R.string.agent_launch_error_hint),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End,
                verticalAlignment = Alignment.CenterVertically) {
                if (detail.isNotBlank()) TextButton(onClick = { detailsOpen = true }) {
                    Text(stringResource(R.string.agent_launch_error_details))
                }
                onRetry?.let { retry ->
                    Spacer(Modifier.width(8.dp))
                    Button(onClick = retry) { Text(uiString(R.string.sftp_retry)) }
                }
            }
        }
    }
    if (detailsOpen) AlertDialog(
        onDismissRequest = { detailsOpen = false },
        title = { Text(stringResource(R.string.agent_launch_error_details)) },
        text = {
            androidx.compose.foundation.text.selection.SelectionContainer {
                Text(detail, style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.heightIn(max = 320.dp).verticalScroll(rememberScrollState()))
            }
        },
        confirmButton = {
            TextButton(onClick = { detailsOpen = false }) { Text(stringResource(R.string.agent_close_detail)) }
        },
        dismissButton = { dev.akko.miaominal.ui.agent.AgentCopyButton(detail) }
    )
}
