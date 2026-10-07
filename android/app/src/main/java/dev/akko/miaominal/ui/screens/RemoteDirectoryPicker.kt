package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.ArrowForward
import androidx.compose.material.icons.outlined.ArrowUpward
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.InsertDriveFile
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.akko.miaominal.ui.components.PageEmptyState
import dev.akko.miaominal.R
import dev.akko.miaominal.data.HostKeyDecision
import dev.akko.miaominal.data.SessionRequestKind
import dev.akko.miaominal.data.SftpDirectoryModel
import dev.akko.miaominal.data.SftpPhase
import dev.akko.miaominal.data.SftpSessionHandle
import dev.akko.miaominal.data.SftpSessionModel
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.SessionRequestDialog
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

const val AgentDirectoryPickerTag = "agent-directory-picker"
const val AgentDirectoryUseTag = "agent-directory-use"

/** Browse directories on the selected SSH host through a short-lived SFTP session. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun RemoteDirectoryPicker(
    hostId: String,
    initialPath: String,
    onDismiss: () -> Unit,
    onSelect: (String) -> Unit,
    selectFile: Boolean = false
) {
    val repository = LocalMiaominalRepository.current
    val scope = rememberCoroutineScope()
    var handle by remember(hostId) { mutableStateOf<SftpSessionHandle?>(null) }
    var session by remember(hostId) { mutableStateOf<SftpSessionModel?>(null) }
    var directory by remember(hostId) {
        mutableStateOf(SftpDirectoryModel("", emptyList(), null, false, 0, 0))
    }
    var pathInput by rememberSaveable(hostId) { mutableStateOf(initialPath.trim().ifBlank { "." }) }
    var openError by remember(hostId) { mutableStateOf<String?>(null) }
    var ownedConnectionId by remember(hostId) { mutableStateOf<Long?>(null) }
    var initialListingRequested by remember(hostId) { mutableStateOf(false) }
    var finishing by remember { mutableStateOf(false) }

    fun finish(selectedPath: String?) {
        if (finishing) return
        finishing = true
        scope.launch {
            ownedConnectionId?.let { repository.closeSftpSession(it) }
            ownedConnectionId = null
            if (selectedPath == null) onDismiss() else onSelect(selectedPath)
        }
    }

    fun list(path: String) {
        val target = path.trim().ifBlank { "." }
        pathInput = target
        handle?.listDirectory(target)?.onFailure { openError = it.message }
    }

    LaunchedEffect(hostId) {
        val reusableIds = repository.sftpSessions.value
            .filter { it.hostId == hostId && !it.phase.isTerminal }
            .mapTo(mutableSetOf(), SftpSessionModel::connectionId)
        repository.openSftpSession(hostId)
            .onSuccess { opened ->
                handle = opened
                session = opened.snapshot()
                if (opened.connectionId !in reusableIds) ownedConnectionId = opened.connectionId
            }
            .onFailure { openError = it.message ?: it.javaClass.simpleName }
    }

    LaunchedEffect(handle) {
        val current = handle ?: return@LaunchedEffect
        while (isActive) {
            session = current.snapshot()
            directory = current.directorySnapshot()
            delay(200)
        }
    }

    LaunchedEffect(handle, session?.phase) {
        if (session?.phase == SftpPhase.Ready && !initialListingRequested) {
            initialListingRequested = true
            list(pathInput)
        }
    }

    LaunchedEffect(directory.requestId, directory.path) {
        if (directory.path.isNotBlank() && directory.error == null) {
            pathInput = directory.path
            openError = null
        }
    }

    Dialog(
        onDismissRequest = { finish(null) },
        properties = DialogProperties(
            usePlatformDefaultWidth = false,
            decorFitsSystemWindows = false
        )
    ) {
        Scaffold(
            modifier = Modifier.fillMaxSize().testTag(AgentDirectoryPickerTag),
            topBar = {
                TopAppBar(
                    title = { Text(stringResource(if (selectFile) R.string.agent_file_picker_title else R.string.agent_directory_picker_title)) },
                    navigationIcon = {
                        IconButton(onClick = { finish(null) }, enabled = !finishing) {
                            Icon(
                                Icons.AutoMirrored.Outlined.ArrowBack,
                                contentDescription = stringResource(R.string.action_back)
                            )
                        }
                    },
                    actions = {
                        if (!selectFile) TextButton(
                            onClick = { finish(directory.path) },
                            enabled = !finishing && !directory.loading && directory.path.isNotBlank() && directory.error == null,
                            modifier = Modifier.testTag(AgentDirectoryUseTag)
                        ) {
                            Text(stringResource(R.string.agent_directory_picker_select))
                        }
                    },
                    colors = TopAppBarDefaults.topAppBarColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainer
                    )
                )
            },
            containerColor = MaterialTheme.colorScheme.surface
        ) { contentPadding ->
            when {
                openError != null && handle == null -> PickerCenteredMessage(
                    text = openError.orEmpty(),
                    modifier = Modifier.padding(contentPadding)
                )
                handle == null || session?.phase != SftpPhase.Ready -> PickerCenteredMessage(
                    text = session?.failure ?: stringResource(R.string.agent_directory_picker_connecting),
                    loading = session?.phase != SftpPhase.Failed,
                    modifier = Modifier.padding(contentPadding)
                )
                else -> Column(Modifier.fillMaxSize().padding(contentPadding)) {
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        IconButton(
                            onClick = { list(parentRemotePath(directory.path.ifBlank { pathInput })) },
                            enabled = directory.path != "/" && !directory.loading
                        ) {
                            Icon(
                                Icons.Outlined.ArrowUpward,
                                contentDescription = stringResource(R.string.sftp_navigate_up)
                            )
                        }
                        OutlinedTextField(
                            value = pathInput,
                            onValueChange = { pathInput = it },
                            label = { Text(stringResource(R.string.agent_directory_picker_path)) },
                            singleLine = true,
                            textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
                            trailingIcon = {
                                IconButton(
                                    onClick = { list(pathInput) },
                                    enabled = pathInput.isNotBlank() && !directory.loading
                                ) {
                                    Icon(
                                        Icons.AutoMirrored.Outlined.ArrowForward,
                                        contentDescription = stringResource(R.string.agent_directory_picker_go)
                                    )
                                }
                            },
                            modifier = Modifier.weight(1f)
                        )
                        IconButton(onClick = { list(directory.path.ifBlank { pathInput }) }) {
                            Icon(Icons.Outlined.Refresh, contentDescription = stringResource(R.string.fm_refresh))
                        }
                    }
                    if (directory.loading) LinearProgressIndicator(Modifier.fillMaxWidth())
                    (directory.error ?: openError)?.let { error ->
                        Text(
                            error,
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodySmall,
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)
                        )
                    }
                    val folders = directory.entries.filter { it.isDirectory || (selectFile && it.kind == dev.akko.miaominal.data.SftpEntryKind.File) }
                    if (folders.isEmpty() && !directory.loading) {
                        PageEmptyState(
                            icon = Icons.Outlined.Folder,
                            title = stringResource(if (selectFile) R.string.agent_file_picker_empty else R.string.agent_directory_picker_empty),
                            modifier = Modifier.weight(1f)
                        )
                    } else {
                        LazyColumn(
                            modifier = Modifier.weight(1f),
                            contentPadding = PaddingValues(vertical = 8.dp)
                        ) {
                            items(folders, key = { it.path }) { entry ->
                                ListItem(
                                    headlineContent = {
                                        Text(entry.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    },
                                    supportingContent = {
                                        Text(entry.path, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    },
                                    leadingContent = { Icon(if (entry.isDirectory) Icons.Outlined.Folder else Icons.Outlined.InsertDriveFile, contentDescription = null) },
                                    modifier = Modifier.clickable(enabled = !finishing && !directory.loading) {
                                        if (entry.isDirectory) list(entry.path) else finish(entry.path)
                                    }
                                )
                                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                            }
                        }
                    }
                }
            }
        }
    }

    session?.request?.let { request ->
        SessionRequestDialog(
            request = request.toSessionRequest(),
            onHostKey = { requestId, decision -> handle?.answerHostKey(requestId, decision) },
            onKeyboardInteractive = { requestId, answers ->
                handle?.answerKeyboardInteractive(requestId, answers)
            },
            onCancel = {
                if (request.kind == SessionRequestKind.HostKey) {
                    handle?.answerHostKey(request.requestId, HostKeyDecision.Reject)
                }
                finish(null)
            }
        )
    }
}

@Composable
private fun PickerCenteredMessage(
    text: String,
    modifier: Modifier = Modifier,
    loading: Boolean = false
) {
    Box(modifier = modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            if (loading) CircularProgressIndicator()
            Text(text, style = MaterialTheme.typography.bodyMedium)
        }
    }
}
