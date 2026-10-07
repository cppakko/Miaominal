package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.layout.ExperimentalLayoutApi

import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.stateDescription
import android.content.ClipData
import android.content.ClipboardManager
import dev.akko.miaominal.data.TransferRequest
import dev.akko.miaominal.data.TransferKey
import dev.akko.miaominal.ui.model.FileSort
import dev.akko.miaominal.ui.model.visibleFiles
import dev.akko.miaominal.ui.components.DirectoryControls
import dev.akko.miaominal.ui.components.PathBreadcrumbs
import dev.akko.miaominal.ui.components.LocalTwoPaneAvailable

import dev.akko.miaominal.ui.uiString

import dev.akko.miaominal.ui.components.WorkspaceBackHandler as BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.InsertDriveFile
import androidx.compose.material.icons.outlined.ArrowUpward
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.CloudDownload
import androidx.compose.material.icons.outlined.CloudUpload
import androidx.compose.material.icons.outlined.CreateNewFolder
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.automirrored.outlined.DriveFileMove
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.FolderOff
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.Pause
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Upload
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Tab
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.MiaominalApplication
import dev.akko.miaominal.ui.components.PageEmptyState
import dev.akko.miaominal.R
import dev.akko.miaominal.data.HostKeyDecision
import dev.akko.miaominal.data.LocalUploadHandle
import dev.akko.miaominal.data.SafLocalEntry
import dev.akko.miaominal.data.SftpDirectoryModel
import dev.akko.miaominal.data.SftpEntryKind
import dev.akko.miaominal.data.SftpOperationErrorModel
import dev.akko.miaominal.data.SftpOperationModel
import dev.akko.miaominal.data.SftpEntryModel
import dev.akko.miaominal.data.SftpPhase
import dev.akko.miaominal.data.SftpSessionHandle
import dev.akko.miaominal.data.SftpSessionModel
import dev.akko.miaominal.data.SftpTransferDirection
import dev.akko.miaominal.data.SftpTransferFailureKindModel
import dev.akko.miaominal.data.SftpTransferHandle
import dev.akko.miaominal.data.SftpTransferModel
import dev.akko.miaominal.data.SftpTransferStateModel
import dev.akko.miaominal.nativebridge.BackendException
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.SessionIdentity
import dev.akko.miaominal.ui.components.SessionRequestDialog
import dev.akko.miaominal.ui.components.SessionTopBarCapsule
import dev.akko.miaominal.ui.model.HostProfileUi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.max

/**
 * Real SFTP browser.
 *
 * The remote pane talks to the native backend only: directory listings carry
 * request ids, transfers are pull-based snapshots, and answers to trust or
 * authentication prompts carry the request id they belong to. The local pane
 * is an authorized SAF tree; uploading and downloading always moves file
 * content through registered descriptors instead of ByteArrays.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun SftpScreen(
    profile: HostProfileUi,
    onDismiss: () -> Unit,
    /** Attach to an already open SFTP session instead of opening a new one. */
    attachConnectionId: Long? = null,
    /** Open the session drawer; the overlay that hosts this screen owns it. */
    onMenu: () -> Unit = {},
    /** Reports the connection id of the session currently displayed. */
    onSessionChanged: (Long?) -> Unit = {},
    modifier: Modifier = Modifier,
    onEditProfile: () -> Unit = {},
    onRequestClose: ((Long) -> Unit)? = null
) {
    val repository = LocalMiaominalRepository.current
    val context = LocalContext.current
    val haptic = androidx.compose.ui.platform.LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val backgroundPolicy = remember(context) {
        (context.applicationContext as? MiaominalApplication)?.container?.backgroundPolicy
    }
    val snackbar = remember { SnackbarHostState() }

    var handle by remember(profile.id) { mutableStateOf<SftpSessionHandle?>(null) }
    var session by remember(profile.id) { mutableStateOf<SftpSessionModel?>(null) }
    var directory by remember(profile.id) { mutableStateOf(SftpDirectoryModel("", emptyList(), null, false, 0, 0)) }
    var transfers by remember(profile.id) { mutableStateOf<List<SftpTransferModel>>(emptyList()) }
    val interruptedTransfers by repository.interruptedSftpTransfers.collectAsState()
    var openError by remember(profile.id) { mutableStateOf<String?>(null) }
    var openAttempt by rememberSaveable(profile.id) { mutableStateOf(0) }
    var lastOperationErrorSequence by rememberSaveable(profile.id) { mutableStateOf(0L) }

    val reportedFailures = remember(profile.id) { mutableStateMapOf<Long, Boolean>() }

    var remotePath by rememberSaveable(profile.id) { mutableStateOf("") }
    var paneName by rememberSaveable(profile.id) { mutableStateOf(FsPane.Remote.name) }
    val pane = FsPane.valueOf(paneName)
    val pagerState = rememberPagerState(initialPage = if (pane == FsPane.Local) 0 else 1, pageCount = { 2 })
    LaunchedEffect(pagerState.currentPage) {
        paneName = if (pagerState.currentPage == 0) FsPane.Local.name else FsPane.Remote.name
    }

    var localTreeUri by rememberSaveable(profile.id) { mutableStateOf<String?>(null) }
    var localPath by rememberSaveable(profile.id) { mutableStateOf("") }
    var localEntries by remember(profile.id) { mutableStateOf<List<SafLocalEntry>>(emptyList()) }
    var localRefresh by remember { mutableStateOf(0) }

    var selectedRemote by rememberSaveable(profile.id) { mutableStateOf(emptyList<String>()) }
    var selectedLocal by rememberSaveable(profile.id) { mutableStateOf(emptyList<String>()) }
    var remoteSelecting by rememberSaveable(profile.id) { mutableStateOf(false) }
    var localSelecting by rememberSaveable(profile.id) { mutableStateOf(false) }
    var remoteQuery by rememberSaveable(profile.id) { mutableStateOf("") }
    var localQuery by rememberSaveable(profile.id) { mutableStateOf("") }
    var remoteSort by rememberSaveable(profile.id) { mutableStateOf(FileSort.Name.name) }
    var localSort by rememberSaveable(profile.id) { mutableStateOf(FileSort.Name.name) }
    val remoteListState = rememberLazyListState()
    val localListState = rememberLazyListState()
    val paneStates = rememberSaveableStateHolder()
    var retryTarget by remember { mutableStateOf<SftpTransferModel?>(null) }
    var recordsRevision by remember { mutableStateOf(0) }
    var confirmCloseTransfers by remember { mutableStateOf(false) }
    var createFolderVisible by remember { mutableStateOf(false) }
    var newFolderName by remember { mutableStateOf("") }
    var renameTarget by remember { mutableStateOf<SftpEntryModel?>(null) }
    var renameInput by remember { mutableStateOf("") }
    var deleteTargets by remember { mutableStateOf<List<SftpEntryModel>?>(null) }
    var pathEditVisible by remember { mutableStateOf(false) }
    var pathInput by remember { mutableStateOf("") }
    var transferSheetVisible by remember { mutableStateOf(false) }
    var pendingDownload by remember { mutableStateOf<List<SftpEntryModel>?>(null) }
    var overwritePrompt by remember { mutableStateOf<List<Pair<SftpEntryModel, String>>?>(null) }
    var overwriteTree by remember { mutableStateOf<String?>(null) }
    var overwriteDirectory by remember { mutableStateOf<String?>(null) }
    var overwriteExistingNames by remember { mutableStateOf<Set<String>>(emptySet()) }
    var localMenuVisible by remember { mutableStateOf(false) }

    suspend fun remoteListing() {
        val sessionHandle = handle ?: return
        val path = remotePath.ifBlank { "." }
        sessionHandle.listDirectory(path).onFailure { snackbar.showSnackbar(describeSftpError(it)) }
    }

    fun selectedRemoteEntries(): List<SftpEntryModel> =
        directory.entries.filter { it.path in selectedRemote }

    fun selectedLocalEntries(): List<SafLocalEntry> =
        localEntries.filter { it.relativePath in selectedLocal }

    // Open one real session per screen instance; a rebuilt page reuses the
    // backend registry through the repository handle. An attached connection
    // (picked from the session drawer) reuses the live session as-is.
    LaunchedEffect(profile.id, openAttempt, attachConnectionId) {
        openError = null
        val attached = attachConnectionId?.let(repository::sftpSessionHandle)?.takeIf { openAttempt == 0 && !it.snapshot().phase.isTerminal }
        if (attached != null) {
            handle = attached
            session = attached.snapshot()
            onSessionChanged(attached.connectionId)
            return@LaunchedEffect
        }
        repository.openSftpSession(profile.id)
            .onSuccess { opened ->
                handle = opened
                session = opened.snapshot()
                onSessionChanged(opened.connectionId)
            }
            .onFailure { error ->
                openError = describeSftpError(error)
            }
    }

    // Pull snapshots while visible; progress merges in the backend, so this
    // only ever reads the latest state.
    LaunchedEffect(handle) {
        val sessionHandle = handle ?: return@LaunchedEffect
        while (isActive) {
            session = sessionHandle.snapshot()
            val listing = sessionHandle.directorySnapshot()
            directory = listing
            transfers = sessionHandle.transfersSnapshot()
            delay(200)
        }
    }

    LaunchedEffect(handle, session?.phase) {
        if (session?.phase == SftpPhase.Ready && remotePath.isBlank() && directory.entries.isEmpty()) {
            handle?.listDirectory(".")?.onFailure { snackbar.showSnackbar(describeSftpError(it)) }
        }
    }

    // Canonicalize the displayed remote path from the accepted listing.
    LaunchedEffect(directory.requestId, directory.path) {
        if (directory.path.isNotBlank() && directory.error == null) {
            remotePath = directory.path
        }
        val selected = selectedRemote.filter { path -> directory.entries.any { it.path == path } }
        if (selected.size != selectedRemote.size) {
            selectedRemote = selected
        }
    }

    LaunchedEffect(directory.requestId, session?.phase) {
        // A late listing for a superseded request is dropped by the backend;
        // the refresh keeps the pane honest after create/rename/delete.
    }

    LaunchedEffect(localTreeUri, localPath, localRefresh) {
        val tree = localTreeUri ?: return@LaunchedEffect
        repository.listSafDirectory(tree, localPath)
            .onSuccess { localEntries = it }
            .onFailure { snackbar.showSnackbar(describeSftpError(it)) }
    }

    // Queue-only directory operations (create/remove/rename) report failure
    // through the typed snapshot; show each failure exactly once.
    LaunchedEffect(session?.operationError?.sequence) {
        val error = session?.operationError ?: return@LaunchedEffect
        if (error.sequence > lastOperationErrorSequence) {
            lastOperationErrorSequence = error.sequence
            snackbar.showSnackbar(describeOperationError(error))
        }
    }

    // Surface transfer failures once per transfer.
    LaunchedEffect(transfers) {
        repository.transferActions.retainFailures(profile.id, transfers)
        transfers.filter { it.state == SftpTransferStateModel.Failed }
            .filterNot { reportedFailures.containsKey(it.transferId) }
            .forEach { failed ->
                reportedFailures[failed.transferId] = true
                snackbar.showSnackbar(describeTransferError(failed))
            }
    }

    fun uniqueLocalTarget(name: String): String {
        val existing = overwriteExistingNames
        val parent = overwriteDirectory ?: localPath
        if (name !in existing) return joinLocalPath(parent, name)
        val dot = name.lastIndexOf('.')
        val base = if (dot > 0) name.substring(0, dot) else name
        val extension = if (dot > 0) name.substring(dot) else ""
        var index = 1
        while (true) {
            val candidate = "$base ($index)$extension"
            if (candidate !in existing) return joinLocalPath(parent, candidate)
            index += 1
        }
    }

    val executeDownloadsWithNames: (List<Pair<SftpEntryModel, String>>) -> Unit = { pairs ->
        val sessionHandle = handle
        val tree = overwriteTree ?: localTreeUri
        overwriteTree = null
        overwriteDirectory = null
        if (sessionHandle != null && tree != null) {
            scope.launch {
                pairs.forEach { (entry, target) ->
                    sessionHandle.download(entry.path, tree, target)
                        .onSuccess { repository.transferActions.record(it, TransferRequest.Download(profile.id, entry.path, tree, target)) }
                        .onFailure { snackbar.showSnackbar(describeSftpError(it)) }
                }
                localRefresh += 1
            }
        }
    }

    val executeDownloads: (List<SftpEntryModel>) -> Unit = { entries ->
        val sessionHandle = handle
        val tree = localTreeUri
        val targetDirectory = localPath
        if (sessionHandle == null) {
            // Nothing to do without a session.
        } else if (tree == null) {
            scope.launch { snackbar.showSnackbar(context.getString(R.string.sftp_download_pick_hint)) }
        } else {
            scope.launch {
                val existing = repository.listSafDirectory(tree, targetDirectory).getOrElse {
                    snackbar.showSnackbar(describeSftpError(it)); return@launch
                }
                val existingNames = existing.map { it.name }.toSet()
                val conflicts = entries.filter { it.name in existingNames }
                if (conflicts.isNotEmpty()) {
                    overwriteTree = tree
                    overwriteDirectory = targetDirectory
                    overwriteExistingNames = existingNames
                    overwritePrompt = entries.map { it to joinLocalPath(targetDirectory, it.name) }
                } else {
                    entries.forEach { entry ->
                        val target = joinLocalPath(targetDirectory, entry.name)
                        sessionHandle.download(entry.path, tree, target)
                            .onSuccess { repository.transferActions.record(it, TransferRequest.Download(profile.id, entry.path, tree, target)) }
                            .onFailure { snackbar.showSnackbar(describeSftpError(it)) }
                    }
                    localRefresh += 1
                }
            }
        }
    }

    val uploadFilePicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenMultipleDocuments()
    ) { uris ->
        backgroundPolicy?.endUserFlow()
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        val sessionHandle = handle ?: return@rememberLauncherForActivityResult
        val targetPath = remotePath.ifBlank { "." }
        scope.launch {
            // Provider queries, descriptor handover, and the Rust registration
            // are all blocking or FFI work: keep them off the main thread.
            withContext(Dispatchers.IO) {
                var queued = 0
                uris.forEach { uri ->
                    val name = context.queryDisplayName(uri) ?: uri.lastPathSegment ?: "upload.bin"
                    repository.registerUploadFile(uri.toString(), name)
                        .onSuccess { local: LocalUploadHandle ->
                            sessionHandle.uploadFile(local, joinRemotePath(targetPath, name))
                                .onSuccess { repository.transferActions.record(it, TransferRequest.UploadFile(profile.id, uri.toString(), name, joinRemotePath(targetPath, name))); queued += 1 }
                                .onFailure { local.close() }
                        }
                        .onFailure { snackbar.showSnackbar(describeSftpError(it)) }
                }
                if (queued > 0) {
                    snackbar.showSnackbar(context.getString(R.string.sftp_queued_transfers, queued))
                }
            }
        }
    }

    val localTreePicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocumentTree()
    ) { uri ->
        backgroundPolicy?.endUserFlow()
        if (uri == null) return@rememberLauncherForActivityResult
        scope.launch {
            repository.persistSafTreePermission(uri.toString())
            localTreeUri = uri.toString()
            localPath = ""
            localRefresh += 1
            val pending = pendingDownload
            pendingDownload = null
            if (pending != null) {
                executeDownloads(pending)
            }
        }
    }

    fun requestLocalTree(download: List<SftpEntryModel>? = null) {
        pendingDownload = download
        backgroundPolicy?.beginUserFlow()
        localTreePicker.launch(localTreeUri?.let(android.net.Uri::parse))
    }

    fun deleteSelection() {
        val sessionHandle = handle ?: return
        val targets = deleteTargets ?: return
        targets.forEach { entry ->
            val result = if (entry.isDirectory) {
                sessionHandle.removeDirectory(entry.path)
            } else {
                sessionHandle.removeFile(entry.path)
            }
            result.onFailure { error ->
                scope.launch { snackbar.showSnackbar(describeSftpError(error)) }
            }
        }
        selectedRemote = emptyList()
        deleteTargets = null
        scope.launch {
            delay(150)
            remoteListing()
        }
    }

    // Transfers from the current backend plus interrupted ones retained after
    // the backend was replaced; the interrupt notice wins for the same id.
    val sheetTransfers = (transfers + repository.transferActions.failureHistory(profile.id) + interruptedTransfers.filter { interrupted ->
        repository.transferActions.request(TransferKey(interrupted.connectionId, interrupted.transferId))?.hostId == profile.id ||
            interrupted.connectionId == handle?.connectionId
    }.filterNot { interrupted ->
        transfers.any { it.transferId == interrupted.transferId && it.connectionId == interrupted.connectionId }
    }).distinctBy { TransferKey(it.connectionId, it.transferId) }
    val visibleTransfers = remember(sheetTransfers, recordsRevision) {
        sheetTransfers.filterNot { repository.transferActions.isHidden(TransferKey(it.connectionId, it.transferId)) }
    }
    val transferFabVisible = true
    fun closeSftp() {
        val connectionId = handle?.connectionId ?: return
        scope.launch {
            repository.closeSftpSession(connectionId).onSuccess {
                handle = null; onSessionChanged(null); onDismiss()
            }.onFailure { snackbar.showSnackbar(describeSftpError(it)) }
        }
    }
    val activeConnectionId = session?.connectionId ?: handle?.connectionId

    // The handle lives in the application-scoped repository, so a rebuilt page
    // can still pause, resume, or cancel an existing transfer.
    fun transferHandleFor(transfer: SftpTransferModel): SftpTransferHandle? =
        repository.sftpTransferHandle(transfer.connectionId, transfer.transferId)

    BackHandler {
        when {
            remoteSelecting -> { selectedRemote = emptyList(); remoteSelecting = false }
            localSelecting -> { selectedLocal = emptyList(); localSelecting = false }
            pane == FsPane.Local && localPath.isNotBlank() -> {
                localPath = parentLocalPath(localPath)
                localRefresh += 1
            }
            else -> onDismiss()
        }
    }

    Scaffold(
        modifier = modifier,
        snackbarHost = { SnackbarHost(snackbar) },
        containerColor = MaterialTheme.colorScheme.surface,
        floatingActionButton = {
            if (transferFabVisible) {
                androidx.compose.material3.ExtendedFloatingActionButton(
                    onClick = { transferSheetVisible = true },
                    text = { Text(stringResource(R.string.ux_transfers,
                        visibleTransfers.count { !it.state.isTerminal },
                        visibleTransfers.count { it.state == SftpTransferStateModel.Failed })) },
                    icon = { Icon(Icons.Outlined.Download, null) }
                )
            }
        }
    ) { contentPadding ->
        Column(modifier = Modifier.fillMaxSize().padding(contentPadding)) {
            SftpConnectionHeader(
                profile = profile,
                session = session,
                onMenu = onMenu,
                onCancel = {
                    if (onRequestClose != null && handle != null) onRequestClose(handle!!.connectionId)
                    else if (transfers.any { !it.state.isTerminal }) confirmCloseTransfers = true
                    else closeSftp()
                }
            )
            val ready = session?.phase == SftpPhase.Ready
            if (ready && !LocalTwoPaneAvailable.current) {
                PrimaryTabRow(selectedTabIndex = if (pane == FsPane.Local) 0 else 1) {
                    Tab(
                        selected = pane == FsPane.Local,
                        onClick = { scope.launch { pagerState.animateScrollToPage(0) } },
                        text = { Text(stringResource(R.string.fm_local)) }
                    )
                    Tab(
                        selected = pane == FsPane.Remote,
                        onClick = { scope.launch { pagerState.animateScrollToPage(1) } },
                        text = { Text(stringResource(R.string.fm_remote)) }
                    )
                }
            }
            val paneContent: @Composable (Int) -> Unit = { page ->
                paneStates.SaveableStateProvider("files-$page") {
                if (!ready) {
                    SftpWaitingPane(session = session, error = openError, onEdit = onEditProfile, onRetry = {
                        openError = null
                        handle = null
                        session = null
                        openAttempt += 1
                    })
                } else {
                when (page) {
                    0 -> LocalPane(
                        selecting = localSelecting,
                        onBeginSelection = { localSelecting = true },
                        query = localQuery, onQuery = { localQuery = it },
                        sort = FileSort.valueOf(localSort), onSort = { localSort = it.name },
                        listState = localListState,
                        transferFabVisible = transferFabVisible,
                        treeUri = localTreeUri,
                        treeLabel = localTreeUri?.let(repository::safTreeLabel),
                        path = localPath,
                        onNavigatePath = { localPath = if (it == ".") "" else it; localQuery = ""; selectedLocal = emptyList(); localRefresh += 1 },
                        entries = localEntries.filter { it.name.contains(localQuery.trim(), true) }.sortedWith(
                            compareByDescending<SafLocalEntry> { it.isDirectory }.then(
                                when (FileSort.valueOf(localSort)) {
                                    FileSort.Name -> compareBy { it.name.lowercase(java.util.Locale.ROOT) }
                                    FileSort.Size -> compareByDescending { it.size ?: Long.MIN_VALUE }
                                    FileSort.Modified -> compareByDescending { it.modifiedAtMs ?: Long.MIN_VALUE }
                                }).thenBy { it.name }),
                        selected = selectedLocal.toSet(),
                        onPickTree = { requestLocalTree() },
                        onNavigateUp = {
                            localPath = parentLocalPath(localPath)
                            localRefresh += 1
                        },
                        onRefresh = { localRefresh += 1 },
                        onEnter = { entry ->
                            localPath = entry.relativePath
                            localRefresh += 1
                        },
                        onToggleSelection = { entry ->
                            localSelecting = true
                            haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.TextHandleMove)
                            selectedLocal = (selectedLocal.toSet()).toggle(entry.relativePath).toList()
                        },
                        onSelectAll = {
                            selectedLocal = localEntries.map { it.relativePath }
                        },
                        onClearSelection = { selectedLocal = emptyList(); localSelecting = false },
                        onUploadToRemote = {
                            val targets = selectedLocalEntries()
                            val sessionHandle = handle
                            if (sessionHandle == null || targets.isEmpty()) return@LocalPane
                            val targetPath = remotePath.ifBlank { "." }
                            val tree = localTreeUri ?: return@LocalPane
                            val sourceDirectory = localPath
                            scope.launch {
                                targets.forEach { entry ->
                                    val full = joinLocalPath(sourceDirectory, entry.name)
                                    sessionHandle.uploadDirectory(
                                        treeUri = tree,
                                        relativePath = full,
                                        label = entry.name,
                                        remotePath = joinRemotePath(targetPath, entry.name)
                                    ).onSuccess { repository.transferActions.record(it, TransferRequest.UploadTree(profile.id, tree, full, entry.name, joinRemotePath(targetPath, entry.name))) }
                                        .onFailure { snackbar.showSnackbar(describeSftpError(it)) }
                                }
                                selectedLocal = emptyList()
                                snackbar.showSnackbar(context.getString(R.string.sftp_queued_transfers, targets.size))
                            }
                        }
                    )

                    else -> RemotePane(
                        selecting = remoteSelecting,
                        onBeginSelection = { remoteSelecting = true },
                        query = remoteQuery, onQuery = { remoteQuery = it },
                        sort = FileSort.valueOf(remoteSort), onSort = { remoteSort = it.name },
                        listState = remoteListState,
                        onNavigatePath = { remotePath = it; remoteQuery = ""; scope.launch { remoteListing() } },
                        transferFabVisible = transferFabVisible,
                        path = remotePath.ifBlank { directory.path },
                        entries = visibleFiles(directory.entries, remoteQuery, FileSort.valueOf(remoteSort)),
                        loading = directory.loading,
                        error = directory.error,
                        selected = selectedRemote.toSet(),
                        onNavigateUp = {
                            val target = parentRemotePath(remotePath)
                            remotePath = target
                            scope.launch { remoteListing() }
                        },
                        onRefresh = { scope.launch { remoteListing() } },
                        onEnter = { entry ->
                            remotePath = entry.path
                            scope.launch { remoteListing() }
                        },
                        onToggleSelection = { entry ->
                            remoteSelecting = true
                            haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.TextHandleMove)
                            selectedRemote = selectedRemote.toSet().toggle(entry.path).toList()
                        },
                        onSelectAll = {
                            selectedRemote = directory.entries.map { it.path }
                        },
                        onClearSelection = { selectedRemote = emptyList(); remoteSelecting = false },
                        onEditPath = {
                            pathInput = remotePath
                            pathEditVisible = true
                        },
                        onCreateFolder = {
                            newFolderName = ""
                            createFolderVisible = true
                        },
                        onRename = { entry ->
                            renameTarget = entry
                            renameInput = entry.name
                        },
                        onDelete = { deleteTargets = selectedRemoteEntries() },
                        onDeleteEntry = { entry -> deleteTargets = listOf(entry) },
                        onUploadFiles = {
                            backgroundPolicy?.beginUserFlow()
                            uploadFilePicker.launch(arrayOf("*/*"))
                        },
                        onUploadFolder = { requestLocalTree() },
                        onDownload = { entries ->
                            paneName = FsPane.Local.name
                            scope.launch { pagerState.animateScrollToPage(0) }
                            if (localTreeUri == null) {
                                requestLocalTree(entries)
                            } else {
                                executeDownloads(entries)
                            }
                        }
                    )
                }
                }
                }
            }
            if (LocalTwoPaneAvailable.current && ready) {
                Row(Modifier.fillMaxSize()) {
                    Box(Modifier.weight(1f).fillMaxSize()) { paneContent(0) }
                    androidx.compose.material3.VerticalDivider()
                    Box(Modifier.weight(1f).fillMaxSize()) { paneContent(1) }
                }
            } else HorizontalPager(state = pagerState, modifier = Modifier.fillMaxSize(),
                userScrollEnabled = ready) { paneContent(it) }
        }
    }

    if (confirmCloseTransfers) AlertDialog(
        onDismissRequest = { confirmCloseTransfers = false },
        title = { Text(stringResource(R.string.ux_close_title)) },
        text = { Text(stringResource(R.string.ux_close_sftp, transfers.count { !it.state.isTerminal })) },
        confirmButton = { TextButton(onClick = { confirmCloseTransfers = false; closeSftp() }) { Text(stringResource(R.string.ux_end_session)) } },
        dismissButton = { TextButton(onClick = { confirmCloseTransfers = false }) { Text(stringResource(R.string.ux_keep_session)) } }
    )
    retryTarget?.let { target ->
        val key = TransferKey(target.connectionId, target.transferId)
        val request = repository.transferActions.request(key)
        var retrying by remember(target.transferId) { mutableStateOf(false) }
        var retryError by remember(target.transferId) { mutableStateOf<String?>(null) }
        AlertDialog(onDismissRequest = { if (!retrying) retryTarget = null },
            title = { Text(stringResource(R.string.ux_retry_transfer)) },
            text = { Column { Text(if (request == null) stringResource(R.string.ux_retry_unavailable)
                else stringResource(R.string.ux_retry_confirm) + "\n" + request.destination)
                retryError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            } },
            confirmButton = { TextButton(enabled = !retrying, onClick = {
                if (request == null) {
                    retryTarget = null
                    if (target.direction == SftpTransferDirection.Upload) {
                        backgroundPolicy?.beginUserFlow(); uploadFilePicker.launch(arrayOf("*/*"))
                    } else requestLocalTree()
                }
                else {
                    retrying = true
                    retryError = null
                    scope.launch {
                        repository.transferActions.retry(key, handle?.connectionId ?: -1L, allowOverwrite = true)
                            .onSuccess { retryTarget = null }
                            .onFailure { retryError = describeSftpError(it) }
                        retrying = false
                    }
                }
            }) { Text(stringResource(if (request == null) R.string.ux_choose else R.string.ux_retry_transfer)) } },
            dismissButton = { Row {
                if (retryError != null) TextButton(enabled = !retrying, onClick = {
                    retryTarget = null
                    if (target.direction == SftpTransferDirection.Upload && request !is TransferRequest.UploadTree) {
                        backgroundPolicy?.beginUserFlow(); uploadFilePicker.launch(arrayOf("*/*"))
                    } else requestLocalTree()
                }) { Text(stringResource(R.string.ux_choose)) }
                TextButton(enabled = !retrying, onClick = { retryTarget = null }) { Text(stringResource(android.R.string.cancel)) }
            } })
    }

    session?.request?.let { request ->
        SessionRequestDialog(
            request = request.toSessionRequest(),
            onHostKey = { requestId, decision ->
                handle?.answerHostKey(requestId, decision)
            },
            onKeyboardInteractive = { requestId, answers ->
                handle?.answerKeyboardInteractive(requestId, answers)
            },
            onCancel = { session?.let { handle?.answerHostKey(it.request?.requestId ?: return@let, HostKeyDecision.Reject) } }
        )
    }

    if (createFolderVisible) {
        AlertDialog(
            onDismissRequest = { createFolderVisible = false },
            title = { Text(stringResource(R.string.fm_new_folder)) },
            text = {
                OutlinedTextField(
                    value = newFolderName,
                    onValueChange = { newFolderName = it },
                    singleLine = true,
                    label = { Text(stringResource(R.string.fm_folder_name)) }
                )
            },
            confirmButton = {
                TextButton(
                    enabled = newFolderName.isNotBlank(),
                    onClick = {
                        val name = newFolderName.trim()
                        createFolderVisible = false
                        scope.launch {
                            handle?.createDirectory(joinRemotePath(remotePath, name))
                                ?.onFailure { snackbar.showSnackbar(describeSftpError(it)) }
                            delay(150)
                            remoteListing()
                        }
                    }
                ) { Text(stringResource(android.R.string.ok)) }
            },
            dismissButton = {
                TextButton(onClick = { createFolderVisible = false }) {
                    Text(stringResource(android.R.string.cancel))
                }
            }
        )
    }

    renameTarget?.let { target ->
        AlertDialog(
            onDismissRequest = { renameTarget = null },
            title = { Text(stringResource(R.string.fm_rename)) },
            text = {
                OutlinedTextField(
                    value = renameInput,
                    label = { Text(stringResource(R.string.fm_rename)) },
                    onValueChange = { renameInput = it },
                    singleLine = true
                )
            },
            confirmButton = {
                TextButton(
                    enabled = renameInput.isNotBlank() && renameInput != target.name,
                    onClick = {
                        val name = renameInput.trim()
                        renameTarget = null
                        val to = joinRemotePath(parentRemotePath(target.path), name)
                        scope.launch {
                            handle?.rename(target.path, to)
                                ?.onFailure { snackbar.showSnackbar(describeSftpError(it)) }
                            delay(150)
                            remoteListing()
                        }
                    }
                ) { Text(stringResource(android.R.string.ok)) }
            },
            dismissButton = {
                TextButton(onClick = { renameTarget = null }) {
                    Text(stringResource(android.R.string.cancel))
                }
            }
        )
    }

    deleteTargets?.let { targets ->
        AlertDialog(
            onDismissRequest = { deleteTargets = null },
            title = {
                Text(
                    stringResource(
                        if (targets.size == 1) R.string.fm_delete_title else R.string.fm_delete_multi_title
                    )
                )
            },
            text = {
                Text(
                    stringResource(
                        R.string.fm_delete_multi_message,
                        stringResource(R.string.fm_remote),
                        targets.size
                    ) + "\n\n" + targets.joinToString("\n") { it.name },
                    modifier = Modifier.heightIn(max = 240.dp).verticalScroll(rememberScrollState())
                        .testTag("sftp-delete-targets")
                )
            },
            confirmButton = {
                TextButton(onClick = { deleteSelection() }) {
                    Text(stringResource(R.string.fm_delete_selected))
                }
            },
            dismissButton = {
                TextButton(onClick = { deleteTargets = null }) {
                    Text(stringResource(android.R.string.cancel))
                }
            }
        )
    }

    if (pathEditVisible) {
        AlertDialog(
            onDismissRequest = { pathEditVisible = false },
            title = { Text(stringResource(R.string.fm_remote_path)) },
            text = {
                OutlinedTextField(
                    value = pathInput,
                    label = { Text(stringResource(R.string.fm_remote_path)) },
                    onValueChange = { pathInput = it },
                    singleLine = true
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        pathEditVisible = false
                        remotePath = pathInput.trim().ifBlank { "." }
                        scope.launch { remoteListing() }
                    }
                ) { Text(stringResource(android.R.string.ok)) }
            },
            dismissButton = {
                Row {
                TextButton(onClick = { (context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as ClipboardManager)
                    .setPrimaryClip(ClipData.newPlainText("path", pathInput)) }) { Text(stringResource(R.string.ux_copy_path)) }
                TextButton(onClick = { pathEditVisible = false }) {
                    Text(stringResource(android.R.string.cancel))
                }
                }
            }
        )
    }

    overwritePrompt?.let { pairs ->
        AlertDialog(
            onDismissRequest = { overwritePrompt = null; overwriteTree = null; overwriteDirectory = null },
            title = { Text(stringResource(R.string.sftp_overwrite_title)) },
            text = {
                Text(stringResource(R.string.sftp_overwrite_message, pairs.firstOrNull()?.first?.name ?: ""))
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        val targets = pairs
                        overwritePrompt = null
                        executeDownloadsWithNames(targets)
                    }
                ) { Text(stringResource(R.string.sftp_overwrite_confirm)) }
            },
            dismissButton = {
                Row {
                    TextButton(
                        onClick = {
                            val targets = pairs
                            overwritePrompt = null
                            executeDownloadsWithNames(
                                targets.map { (entry, _) -> entry to uniqueLocalTarget(entry.name) }
                            )
                        }
                    ) { Text(stringResource(R.string.sftp_save_as)) }
                    TextButton(onClick = { overwritePrompt = null; overwriteTree = null; overwriteDirectory = null }) {
                        Text(stringResource(android.R.string.cancel))
                    }
                }
            }
        )
    }

    if (transferSheetVisible) {
        ModalBottomSheet(onDismissRequest = { transferSheetVisible = false }) {
            Column(modifier = Modifier.fillMaxWidth().padding(16.dp)) {
                Text(
                    stringResource(R.string.sftp_transfers_title),
                    style = MaterialTheme.typography.titleMedium
                )
                Spacer(Modifier.height(8.dp))
                TextButton(onClick = { repository.transferActions.hideCompleted(sheetTransfers); recordsRevision += 1 }) {
                    Text(stringResource(R.string.ux_clear_completed))
                }
                LazyColumn {
                    items(visibleTransfers, key = { "${it.connectionId}:${it.transferId}" }) { transfer ->
                        SftpTransferRow(
                            transfer = transfer,
                            onRetry = { retryTarget = transfer },
                            handle = transferHandleFor(transfer),
                            onPause = { transferHandleFor(transfer)?.pause() },
                            onResume = { transferHandleFor(transfer)?.resume() },
                            onCancel = { transferHandleFor(transfer)?.cancel() }
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun SftpConnectionHeader(
    profile: HostProfileUi,
    session: SftpSessionModel?,
    onMenu: () -> Unit,
    onCancel: () -> Unit
) {
    val connected = session?.phase == SftpPhase.Ready
    Column(
        Modifier.fillMaxWidth()
    ) {
        // Same layout as the terminal top bar: drawer affordance, host identity
        // with the status dot, then the session actions.
        SessionTopBarCapsule {
            IconButton(onClick = onMenu) {
                Icon(
                    Icons.Outlined.Menu,
                    contentDescription = stringResource(R.string.links_sidebar_toggle),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            SessionIdentity(profile, connected, Modifier.weight(1f))
            if (connected) {
                IconButton(onClick = onCancel) {
                    Icon(
                        Icons.Outlined.Close,
                        contentDescription = stringResource(R.string.terminal_close_session),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        }
    }
}

private fun phaseLabel(session: SftpSessionModel): String = when (session.phase) {
    SftpPhase.Connecting -> uiString(R.string.ui_connecting_to, session.hopLabel ?: "")
    SftpPhase.WaitingForTrust -> uiString(R.string.ui_awaiting_host_key_verification)
    SftpPhase.WaitingForAuthentication -> uiString(R.string.forward_runtime_waiting)
    SftpPhase.OpeningSubsystem -> uiString(R.string.ui_opening_sftp_subsystem)
    SftpPhase.Ready -> uiString(R.string.ui_connected_to, session.hostLabel)
    SftpPhase.Closing -> uiString(R.string.ui_closing)
    SftpPhase.Closed -> uiString(R.string.ui_connection_closed)
    SftpPhase.Failed -> uiString(R.string.ui_connection_failed)
}

@Composable
private fun SftpWaitingPane(session: SftpSessionModel?, error: String?, onEdit: () -> Unit, onRetry: () -> Unit) {
    val failed = error != null || session?.phase?.isTerminal == true
    if (failed) {
        PageEmptyState(
            icon = Icons.Outlined.FolderOff,
            title = stringResource(R.string.ui_connection_failed),
            text = error ?: session?.failure ?: session?.let(::phaseLabel).orEmpty(),
            modifier = Modifier.fillMaxSize(),
            action = { dev.akko.miaominal.ui.components.ConnectionErrorActions(
                error ?: session?.failure.orEmpty(), onRetry, onEdit) }
        )
    } else Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            CircularProgressIndicator()
            Spacer(Modifier.height(12.dp))
            Text(session?.let(::phaseLabel) ?: uiString(R.string.ui_opening_sftp_session))
        }
    }
}

@OptIn(ExperimentalFoundationApi::class, ExperimentalMaterial3Api::class)
@Composable
private fun RemotePane(
    selecting: Boolean, onBeginSelection: () -> Unit,
    query: String, onQuery: (String) -> Unit,
    sort: FileSort, onSort: (FileSort) -> Unit,
    listState: LazyListState,
    onNavigatePath: (String) -> Unit,
    transferFabVisible: Boolean,
    path: String,
    entries: List<SftpEntryModel>,
    loading: Boolean,
    error: String?,
    selected: Set<String>,
    onNavigateUp: () -> Unit,
    onRefresh: () -> Unit,
    onEnter: (SftpEntryModel) -> Unit,
    onToggleSelection: (SftpEntryModel) -> Unit,
    onSelectAll: () -> Unit,
    onClearSelection: () -> Unit,
    onEditPath: () -> Unit,
    onCreateFolder: () -> Unit,
    onRename: (SftpEntryModel) -> Unit,
    onDelete: () -> Unit,
    onDeleteEntry: (SftpEntryModel) -> Unit,
    onUploadFiles: () -> Unit,
    onUploadFolder: () -> Unit,
    onDownload: (List<SftpEntryModel>) -> Unit
) {
    var menuVisible by remember { mutableStateOf(false) }
    var detail by remember { mutableStateOf<SftpEntryModel?>(null) }
    val context = LocalContext.current
    val selectedLabel = stringResource(R.string.ux_selected)
    val unselectedLabel = stringResource(R.string.ux_not_selected)
    detail?.let { entry ->
        ModalBottomSheet(onDismissRequest = { detail = null }) {
            Column(Modifier.fillMaxWidth().padding(24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(entry.name, style = MaterialTheme.typography.headlineSmall)
                androidx.compose.foundation.text.selection.SelectionContainer { Text(entry.path) }
                Text(formatBytes(entry.size), style = MaterialTheme.typography.bodyMedium)
                TextButton(onClick = { detail = null; onDownload(listOf(entry)) }) { Text(stringResource(R.string.fm_download)) }
                TextButton(onClick = { detail = null; onRename(entry) }) { Text(stringResource(R.string.fm_rename)) }
                TextButton(onClick = { detail = null; onDeleteEntry(entry) }) { Text(stringResource(R.string.action_delete)) }
                TextButton(onClick = { (context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as ClipboardManager)
                    .setPrimaryClip(ClipData.newPlainText("path", entry.path)) }) { Text(stringResource(R.string.ux_copy_path)) }
            }
        }
    }
    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 4.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            IconButton(onClick = onNavigateUp) {
                Icon(Icons.Outlined.ArrowUpward, contentDescription = stringResource(R.string.sftp_navigate_up))
            }
            Box(
                modifier = Modifier.weight(1f)
                    .clickable(
                        role = androidx.compose.ui.semantics.Role.Button,
                        onClickLabel = stringResource(R.string.sftp_edit_path),
                        onClick = onEditPath
                    )
                    .heightIn(min = 48.dp)
                    .padding(horizontal = 4.dp),
                contentAlignment = Alignment.CenterStart
            ) {
                Text(
                    text = path.ifBlank { "." },
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Monospace,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
            }
            IconButton(onClick = onCreateFolder) {
                Icon(Icons.Outlined.CreateNewFolder, contentDescription = stringResource(R.string.fm_new_folder))
            }
            if (error == null) IconButton(onClick = onRefresh, enabled = !loading) {
                Icon(Icons.Outlined.Refresh, contentDescription = stringResource(R.string.fm_refresh))
            }
            Box {
                IconButton(onClick = { menuVisible = true }) {
                    Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.fm_more_actions))
                }
                DropdownMenu(expanded = menuVisible, onDismissRequest = { menuVisible = false }) {
                    DropdownMenuItem(text = { Text(stringResource(R.string.ux_choose)) }, onClick = { menuVisible = false; onBeginSelection() })
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.sftp_upload_files)) },
                        onClick = { menuVisible = false; onUploadFiles() },
                        leadingIcon = { Icon(Icons.Outlined.Upload, contentDescription = null) }
                    )
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.sftp_upload_folder)) },
                        onClick = { menuVisible = false; onUploadFolder() },
                        leadingIcon = { Icon(Icons.Outlined.CloudUpload, contentDescription = null) }
                    )
                }
            }
        }
        PathBreadcrumbs(path, onNavigatePath)
        DirectoryControls(query, onQuery, sort, onSort, onBeginSelection)
        if (loading) {
            LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
        }
        if (selecting || selected.isNotEmpty()) {
            SelectionBar(
                count = selected.size,
                onSelectAll = onSelectAll,
                onClear = onClearSelection,
                actions = {
                    TextButton(onClick = { onDownload(entries.filter { it.path in selected }) }) {
                        Text(stringResource(R.string.fm_download))
                    }
                    var selectionMenu by remember { mutableStateOf(false) }
                    Box {
                        IconButton(onClick = { selectionMenu = true }) { Icon(Icons.Outlined.MoreVert, stringResource(R.string.fm_more_actions)) }
                        DropdownMenu(selectionMenu, { selectionMenu = false }) {
                            DropdownMenuItem(text = { Text(stringResource(R.string.fm_delete_selected)) }, onClick = { selectionMenu = false; onDelete() })
                        }
                    }
                }
            )
        }
        if (error != null) {
            PageEmptyState(Icons.Outlined.FolderOff, stringResource(R.string.ui_could_not_open_directory),
                text = error,
                modifier = Modifier.fillMaxSize(),
                action = { Button(onClick = onRefresh, enabled = !loading) { Text(stringResource(R.string.sftp_retry)) } })
        } else if (entries.isEmpty() && !loading) {
            PageEmptyState(Icons.Outlined.FolderOpen, stringResource(R.string.sftp_no_entries),
                modifier = Modifier.fillMaxSize())
        } else {
            LazyColumn(state = listState, contentPadding = PaddingValues(bottom = if (transferFabVisible) 112.dp else 0.dp)) {
                items(entries, key = { it.path }) { entry ->
                    RemoteEntryRow(
                        entry = entry,
                        selected = entry.path in selected,
                        onClick = {
                            when (dev.akko.miaominal.ui.model.fileTapAction(selecting || selected.isNotEmpty(), entry.isDirectory)) {
                                dev.akko.miaominal.ui.model.FileTapAction.ToggleSelection -> onToggleSelection(entry)
                                dev.akko.miaominal.ui.model.FileTapAction.OpenDirectory -> onEnter(entry)
                                dev.akko.miaominal.ui.model.FileTapAction.OpenDetails -> detail = entry
                            }
                        },
                        onLongClick = { onToggleSelection(entry) },
                        onRename = { onRename(entry) },
                        onDelete = { onDeleteEntry(entry) },
                        onDownload = { onDownload(listOf(entry)) }
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun RemoteEntryRow(
    entry: SftpEntryModel,
    selected: Boolean,
    onClick: () -> Unit,
    onLongClick: () -> Unit,
    onRename: () -> Unit,
    onDelete: () -> Unit,
    onDownload: () -> Unit
) {
    var menuVisible by remember { mutableStateOf(false) }
    ListItem(
        colors = if (selected) {
            ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.secondaryContainer)
        } else {
            ListItemDefaults.colors()
        },
        modifier = Modifier.combinedClickable(onClick = onClick, onLongClick = onLongClick,
            onLongClickLabel = stringResource(R.string.ux_selection)).semantics { this.selected = selected },
        headlineContent = {
            Text(entry.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
        },
        supportingContent = {
            Text(
                stringResource(
                    R.string.fm_entry_metadata,
                    if (entry.isDirectory) uiString(R.string.ui_folder) else formatBytes(entry.size),
                    entry.owner ?: entry.attributes ?: ""
                ),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        },
        leadingContent = {
            if (selected) Icon(Icons.Outlined.CheckCircle, stringResource(R.string.ux_selected))
            else Icon(entryIcon(entry), contentDescription = null)
        },
        trailingContent = {
            Box {
                IconButton(onClick = { menuVisible = true }) {
                    Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.fm_item_actions))
                }
                DropdownMenu(expanded = menuVisible, onDismissRequest = { menuVisible = false }) {
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.fm_download)) },
                        onClick = { menuVisible = false; onDownload() },
                        leadingIcon = { Icon(Icons.Outlined.Download, contentDescription = null) }
                    )
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.fm_rename)) },
                        onClick = { menuVisible = false; onRename() },
                        leadingIcon = { Icon(Icons.AutoMirrored.Outlined.DriveFileMove, contentDescription = null) }
                    )
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.action_delete)) },
                        onClick = { menuVisible = false; onDelete() },
                        leadingIcon = { Icon(Icons.Outlined.Delete, contentDescription = null) }
                    )
                }
            }
        }
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LocalPane(
    selecting: Boolean, onBeginSelection: () -> Unit,
    query: String, onQuery: (String) -> Unit,
    sort: FileSort, onSort: (FileSort) -> Unit,
    listState: LazyListState,
    transferFabVisible: Boolean,
    treeUri: String?,
    treeLabel: String?,
    path: String,
    onNavigatePath: (String) -> Unit,
    entries: List<SafLocalEntry>,
    selected: Set<String>,
    onPickTree: () -> Unit,
    onNavigateUp: () -> Unit,
    onRefresh: () -> Unit,
    onEnter: (SafLocalEntry) -> Unit,
    onToggleSelection: (SafLocalEntry) -> Unit,
    onSelectAll: () -> Unit,
    onClearSelection: () -> Unit,
    onUploadToRemote: () -> Unit
) {
    var detail by remember { mutableStateOf<SafLocalEntry?>(null) }
    val context = LocalContext.current
    detail?.let { entry -> ModalBottomSheet(onDismissRequest = { detail = null }) {
        Column(Modifier.fillMaxWidth().padding(24.dp)) {
            Text(entry.name, style = MaterialTheme.typography.headlineSmall)
            androidx.compose.foundation.text.selection.SelectionContainer { Text(entry.relativePath) }
            TextButton(onClick = { detail = null; onToggleSelection(entry); onUploadToRemote() }) { Text(stringResource(R.string.fm_upload_to_remote)) }
            TextButton(onClick = { detail = null; onToggleSelection(entry) }) { Text(stringResource(R.string.ux_choose)) }
            TextButton(onClick = { (context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as ClipboardManager)
                .setPrimaryClip(ClipData.newPlainText("path", entry.relativePath)) }) { Text(stringResource(R.string.ux_copy_path)) }
        }
    } }
    Column(modifier = Modifier.fillMaxSize()) {
        if (treeUri == null) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Icon(Icons.Outlined.FolderOpen, contentDescription = null)
                    Spacer(Modifier.height(8.dp))
                    Text(stringResource(R.string.sftp_local_pick_hint))
                    Spacer(Modifier.height(12.dp))
                    Button(onClick = onPickTree) { Text(stringResource(R.string.sftp_local_pick)) }
                }
            }
            return@Column
        }
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 4.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            IconButton(onClick = onNavigateUp) {
                Icon(Icons.Outlined.ArrowUpward, contentDescription = stringResource(R.string.sftp_navigate_up))
            }
            Text(
                text = if (path.isBlank()) (treeLabel ?: "") else path,
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f)
            )
            IconButton(onClick = onRefresh) {
                Icon(Icons.Outlined.Refresh, contentDescription = stringResource(R.string.fm_refresh))
            }
            IconButton(onClick = onPickTree) {
                Icon(Icons.Outlined.FolderOpen, contentDescription = stringResource(R.string.sftp_local_pick))
            }
        }
        PathBreadcrumbs(path, onNavigatePath)
        DirectoryControls(query, onQuery, sort, onSort, onBeginSelection)
        if (selecting || selected.isNotEmpty()) {
            SelectionBar(
                count = selected.size,
                onSelectAll = onSelectAll,
                onClear = onClearSelection,
                actions = {
                    TextButton(onClick = onUploadToRemote) {
                        Text(stringResource(R.string.fm_upload_to_remote))
                    }
                }
            )
        }
        if (entries.isEmpty()) {
            PageEmptyState(Icons.Outlined.FolderOpen, stringResource(R.string.fm_local_empty),
                modifier = Modifier.fillMaxSize())
        } else {
            LazyColumn(state = listState, contentPadding = PaddingValues(bottom = if (transferFabVisible) 112.dp else 0.dp)) {
                items(entries, key = { it.relativePath }) { entry ->
                    ListItem(
                        colors = if (entry.relativePath in selected) {
                            ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.secondaryContainer)
                        } else {
                            ListItemDefaults.colors()
                        },
                        modifier = Modifier.combinedClickable(
                            onClick = { if (selecting || selected.isNotEmpty()) onToggleSelection(entry)
                                else if (entry.isDirectory) onEnter(entry) else detail = entry },
                            onLongClick = { onToggleSelection(entry) },
                            onLongClickLabel = stringResource(R.string.ux_selection)
                        ).semantics { this.selected = entry.relativePath in selected },
                        headlineContent = { Text(entry.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        supportingContent = {
                            Text(if (entry.isDirectory) uiString(R.string.ui_folder) else formatBytes(entry.size))
                        },
                        leadingContent = {
                            Icon(
                                if (entry.isDirectory) Icons.Outlined.Folder else Icons.AutoMirrored.Outlined.InsertDriveFile,
                                contentDescription = null
                            )
                        },
                        trailingContent = {
                            if (entry.relativePath in selected) {
                                Icon(Icons.Outlined.CheckCircle, contentDescription = null)
                            }
                        }
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun SelectionBar(
    count: Int,
    onSelectAll: () -> Unit,
    onClear: () -> Unit,
    actions: @Composable () -> Unit
) {
    Surface(color = MaterialTheme.colorScheme.secondaryContainer) {
        androidx.compose.foundation.layout.FlowRow(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            Text(
                text = stringResource(R.string.fm_selected_count, count),
                style = MaterialTheme.typography.labelLarge,
                modifier = Modifier.padding(start = 4.dp)
            )
            TextButton(onClick = onSelectAll) { Text(stringResource(R.string.fm_select_all)) }
            actions()
            IconButton(onClick = onClear) {
                Icon(Icons.Outlined.Close, contentDescription = stringResource(R.string.fm_clear_selection))
            }
        }
    }
}

@Composable
private fun SftpTransferRow(
    transfer: SftpTransferModel,
    onRetry: () -> Unit,
    handle: SftpTransferHandle?,
    onPause: () -> Unit,
    onResume: () -> Unit,
    onCancel: () -> Unit
) {
    Column(modifier = Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(transferIcon(transfer), contentDescription = null)
            Spacer(Modifier.width(8.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    transfer.sourceLabel.ifBlank { transfer.destination },
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
                Text(
                    "${transferStateLabel(transfer.state)} · ${formatBytes(transfer.bytesComplete)} / ${formatBytes(transfer.bytesTotal)}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            when (transfer.state) {
                SftpTransferStateModel.Running, SftpTransferStateModel.Queued -> {
                    IconButton(onClick = onPause) {
                        Icon(Icons.Outlined.Pause, contentDescription = uiString(R.string.ui_pause))
                    }
                }
                SftpTransferStateModel.Paused -> {
                    IconButton(onClick = onResume) {
                        Icon(Icons.Outlined.PlayArrow, contentDescription = uiString(R.string.ui_resume))
                    }
                }
                else -> Unit
            }
            if (!transfer.state.isTerminal) {
                IconButton(onClick = onCancel) {
                    Icon(Icons.Outlined.Close, contentDescription = uiString(R.string.tool_action_cancel))
                }
            }
        }
        LinearProgressIndicator(
            progress = { transfer.progress },
            modifier = Modifier.fillMaxWidth().padding(top = 4.dp)
        )
        if (transfer.childCount > 0) {
            Text(
                text = uiString(R.string.ui_items, transfer.childrenDone, transfer.childCount) +
                    if (transfer.childrenFailed > 0) uiString(R.string.ui_failed, transfer.childrenFailed) else "",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        if (transfer.state == SftpTransferStateModel.Failed) {
            TextButton(onClick = onRetry) { Text(stringResource(R.string.ux_retry_transfer)) }
        }
        if (transfer.error != null) {
            Text(
                describeTransferError(transfer),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error
            )
        }
        HorizontalDivider(color = Color.Transparent)
    }
}

private fun <T> Set<T>.toggle(value: T): Set<T> =
    if (value in this) this - value else this + value

private enum class FsPane {
    Local,
    Remote
}

internal fun joinRemotePath(parent: String, name: String): String = when {
    parent.isBlank() || parent == "." -> if (parent == ".") "./$name" else name
    parent.endsWith("/") -> "$parent$name"
    else -> "$parent/$name"
}

internal fun parentRemotePath(path: String): String {
    val trimmed = path.trimEnd('/')
    if (trimmed.isEmpty() || trimmed == "/") return "/"
    val index = trimmed.lastIndexOf('/')
    return when {
        index < 0 -> "."
        index == 0 -> "/"
        else -> trimmed.substring(0, index)
    }
}

private fun joinLocalPath(parent: String, name: String): String =
    if (parent.isBlank()) name else "$parent/$name"

private fun parentLocalPath(path: String): String =
    path.substringBeforeLast('/', "")

private fun transferPercent(transfers: List<SftpTransferModel>): Int {
    if (transfers.isEmpty()) return 0
    val total = transfers.map { it.progress }.fold(0f) { sum, value -> sum + value }
    return (total / transfers.size * 100f).toInt().coerceIn(0, 100)
}

private fun describeSftpError(error: Throwable): String = when (error) {
    is dev.akko.miaominal.data.TransferRetryException -> uiString(when (error.reason) {
        dev.akko.miaominal.data.TransferRetryFailure.InProgress -> R.string.ux_retry_inprogress
        dev.akko.miaominal.data.TransferRetryFailure.SourceUnavailable -> R.string.ux_retry_sourceunavailable
        dev.akko.miaominal.data.TransferRetryFailure.Reconnect -> R.string.ux_retry_reconnect
        dev.akko.miaominal.data.TransferRetryFailure.OriginalHost -> R.string.ux_retry_originalhost
        dev.akko.miaominal.data.TransferRetryFailure.DestinationCheck -> R.string.ux_retry_destinationcheck
        dev.akko.miaominal.data.TransferRetryFailure.ConfirmReplacement -> R.string.ux_retry_confirmreplacement
        dev.akko.miaominal.data.TransferRetryFailure.NotFailed -> R.string.ux_retry_notfailed
    })
    is BackendException.VaultLocked -> uiString(R.string.ui_vault_locked_unlock_it_and_retry)
    is BackendException.PermissionDenied -> uiString(R.string.ui_permission_denied_cannot_access_this_location)
    is BackendException.NoSpace -> uiString(R.string.ui_not_enough_space_on_the_destination)
    is BackendException.Unsupported -> uiString(R.string.ui_provider_does_not_support_this_operation, error.capability)
    is BackendException.LocalHandleInvalid -> uiString(R.string.ui_local_file_access_expired_select_the_file_again)
    is BackendException.Network -> uiString(R.string.ui_network_interrupted, error.networkFailure)
    is BackendException.SessionNotReady -> uiString(R.string.ui_sftp_session_is_not_ready)
    is BackendException.RequestExpired -> uiString(R.string.ui_this_authentication_request_has_expired)
    is BackendException.AuthenticationFailed -> uiString(R.string.ui_authentication_failed, error.authenticationFailure)
    is BackendException.TrustPending -> uiString(R.string.ui_verify_the_host_key_first)
    is BackendException.Closed -> uiString(R.string.ui_session_closed)
    else -> error.message?.takeIf(String::isNotBlank) ?: error.javaClass.simpleName
}

private fun describeOperationError(error: SftpOperationErrorModel): String {
    val operation = when (error.operation) {
        SftpOperationModel.CreateDirectory -> uiString(R.string.fm_new_folder)
        SftpOperationModel.RemoveFile, SftpOperationModel.RemoveDirectory -> uiString(R.string.action_delete)
        SftpOperationModel.Rename -> uiString(R.string.keychain_rename)
        SftpOperationModel.ListDirectory, SftpOperationModel.ListSubdirectory -> uiString(R.string.ui_read_directory)
    }
    val target = error.path?.let { " $it" } ?: ""
    val lowered = error.message.lowercase()
    val reason = when {
        lowered.contains("permission denied") || lowered.contains("os error 13") -> uiString(R.string.ui_permission_denied)
        lowered.contains("no space") || lowered.contains("os error 28") -> uiString(R.string.ui_not_enough_space)
        lowered.contains("no such file") -> uiString(R.string.ui_item_no_longer_exists)
        lowered.contains("unsupported") || lowered.contains("does not support") -> uiString(R.string.ui_provider_does_not_support_this_operation_182)
        else -> null
    }
    return if (reason == null) uiString(R.string.ui_failed_183, operation, target, error.message) else uiString(R.string.ui_failed_183, operation, target, reason)
}

private fun describeTransferError(transfer: SftpTransferModel): String {
    val detail = transfer.error?.takeIf(String::isNotBlank)
    val label = when (transfer.failureKind) {
        SftpTransferFailureKindModel.Permission -> uiString(R.string.ui_permission_denied)
        SftpTransferFailureKindModel.NoSpace -> uiString(R.string.ui_not_enough_space)
        SftpTransferFailureKindModel.ProviderRefused -> uiString(R.string.ui_provider_rejected_the_operation_try_saving_with_a_new_name)
        SftpTransferFailureKindModel.LocalHandle -> uiString(R.string.ui_local_file_access_expired)
        SftpTransferFailureKindModel.Network -> uiString(R.string.ui_network_interrupted_186)
        else -> uiString(R.string.ui_transfer_failed)
    }
    return if (detail == null) label else "$label：$detail"
}

private fun formatBytes(bytes: Long?): String {
    if (bytes == null) return "—"
    if (bytes < 1024) return "$bytes B"
    val units = listOf("KB", "MB", "GB", "TB")
    var value = bytes.toDouble()
    var unit = -1
    while (value >= 1024 && unit < units.lastIndex) {
        value /= 1024
        unit += 1
    }
    return if (value >= 100) {
        "${value.toInt()} ${units[unit]}"
    } else {
        String.format(java.util.Locale.US, "%.1f %s", value, units[unit])
    }
}

private fun entryIcon(entry: SftpEntryModel): ImageVector = when (entry.kind) {
    SftpEntryKind.Directory -> Icons.Outlined.Folder
    SftpEntryKind.Symlink -> Icons.AutoMirrored.Outlined.DriveFileMove
    else -> Icons.AutoMirrored.Outlined.InsertDriveFile
}

private fun transferIcon(transfer: SftpTransferModel): ImageVector =
    if (transfer.direction == SftpTransferDirection.Upload) {
        Icons.Outlined.CloudUpload
    } else {
        Icons.Outlined.CloudDownload
    }

private fun transferStateLabel(state: SftpTransferStateModel): String = when (state) {
    SftpTransferStateModel.Queued -> uiString(R.string.ui_queued)
    SftpTransferStateModel.Running -> uiString(R.string.ui_transferring)
    SftpTransferStateModel.Paused -> uiString(R.string.ui_paused)
    SftpTransferStateModel.Done -> uiString(R.string.ui_completed)
    SftpTransferStateModel.Cancelled -> uiString(R.string.ui_cancelled)
    SftpTransferStateModel.Failed -> uiString(R.string.forward_runtime_failed)
}

private fun android.content.Context.queryDisplayName(uri: android.net.Uri): String? =
    runCatching {
        contentResolver.query(uri, arrayOf(android.provider.OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getString(0) else null
            }
    }.getOrNull()

private fun maxOfOrNull(values: List<Long?>): Long? =
    values.filterNotNull().maxOrNull()
