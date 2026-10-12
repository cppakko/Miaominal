package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import android.content.ClipData
import android.content.ClipboardManager
import dev.akko.miaominal.data.TransferRequest
import dev.akko.miaominal.data.TransferKey
import dev.akko.miaominal.ui.model.FileSort
import dev.akko.miaominal.ui.model.visibleFiles
import dev.akko.miaominal.ui.model.visibleLocalFiles
import dev.akko.miaominal.ui.components.LocalTwoPaneAvailable
import dev.akko.miaominal.ui.components.SessionTopBarCapsule

import dev.akko.miaominal.ui.uiString

import dev.akko.miaominal.ui.components.WorkspaceBackHandler as BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxHeight
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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.CheckCircle
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
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.SelectAll
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.SwapVert
import androidx.compose.material.icons.outlined.Upload
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import dev.akko.miaominal.ui.theme.UiSpacing
import dev.akko.miaominal.ui.theme.successColor
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.withFrameNanos
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.MiaominalApplication
import dev.akko.miaominal.R
import dev.akko.miaominal.data.HostKeyDecision
import dev.akko.miaominal.data.LocalUploadHandle
import dev.akko.miaominal.data.SafLocalEntry
import dev.akko.miaominal.data.isSharedStorageTree
import dev.akko.miaominal.ui.components.rememberAllFilesAccessRequest
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
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
import dev.akko.miaominal.ui.components.SessionRequestDialog
import dev.akko.miaominal.ui.model.HostProfileUi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlin.math.abs
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
    LaunchedEffect(pagerState.settledPage) {
        paneName = if (pagerState.settledPage == 0) FsPane.Local.name else FsPane.Remote.name
    }
    LaunchedEffect(pagerState, pagerState.isScrollInProgress) {
        if (!pagerState.isScrollInProgress) {
            // Let the normal release/fling start before recovering a cancelled
            // scroll or a restored drag offset that would otherwise stay between panes.
            withFrameNanos { }
            if (!pagerState.isScrollInProgress && abs(pagerState.currentPageOffsetFraction) > 0.001f) {
                pagerState.requestScrollToPage(pagerState.currentPage)
            }
        }
    }

    var localTreeUri by rememberSaveable(profile.id) { mutableStateOf(repository.defaultLocalTreeUri()) }
    var localPath by rememberSaveable(profile.id) { mutableStateOf("") }
    var localEntries by remember(profile.id) { mutableStateOf<List<SafLocalEntry>>(emptyList()) }
    var localRefresh by remember { mutableStateOf(0) }
    var localLoading by remember { mutableStateOf(false) }
    var localError by remember { mutableStateOf<String?>(null) }
    var listedLocalRoot by remember { mutableStateOf<String?>(null) }
    var listedLocalPath by remember { mutableStateOf("") }

    var selectedRemote by rememberSaveable(profile.id) { mutableStateOf(emptyList<String>()) }
    var selectedLocal by rememberSaveable(profile.id) { mutableStateOf(emptyList<String>()) }
    var remoteSelecting by rememberSaveable(profile.id) { mutableStateOf(false) }
    var localSelecting by rememberSaveable(profile.id) { mutableStateOf(false) }
    var remoteQuery by rememberSaveable(profile.id) { mutableStateOf("") }
    var localQuery by rememberSaveable(profile.id) { mutableStateOf("") }
    var remoteSearchVisible by rememberSaveable(profile.id) { mutableStateOf(false) }
    var localSearchVisible by rememberSaveable(profile.id) { mutableStateOf(false) }
    var remoteSort by rememberSaveable(profile.id) { mutableStateOf(FileSort.Name.name) }
    var localSort by rememberSaveable(profile.id) { mutableStateOf(FileSort.Name.name) }
    var remoteSortAscending by rememberSaveable(profile.id) { mutableStateOf(true) }
    var localSortAscending by rememberSaveable(profile.id) { mutableStateOf(true) }
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
        sessionHandle.listDirectory(path)
            .onSuccess { directory = sessionHandle.directorySnapshot() }
            .onFailure { snackbar.showSnackbar(describeSftpError(it)) }
    }

    fun navigateRemote(path: String) {
        val sessionHandle = handle ?: return
        remotePath = path
        remoteQuery = ""; remoteSearchVisible = false
        selectedRemote = emptyList(); remoteSelecting = false
        remoteListState.requestScrollToItem(0)
        sessionHandle.listDirectory(path)
            .onSuccess { directory = sessionHandle.directorySnapshot() }
            .onFailure {
                remotePath = directory.path
                scope.launch { snackbar.showSnackbar(describeSftpError(it)) }
            }
    }

    fun navigateLocal(path: String) {
        localPath = if (path == ".") "" else path
        localQuery = ""; localSearchVisible = false
        selectedLocal = emptyList(); localSelecting = false
        localListState.requestScrollToItem(0)
        localRefresh += 1
    }

    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, repository) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME && (localTreeUri == null || isSharedStorageTree(localTreeUri))) {
                val availableRoot = repository.defaultLocalTreeUri()
                if (localTreeUri != availableRoot) {
                    localTreeUri = availableRoot
                    localEntries = emptyList()
                    navigateLocal("")
                }
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
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
    LaunchedEffect(directory.requestId, directory.path, directory.loading) {
        if (!directory.loading && directory.path.isNotBlank() && directory.error == null) {
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
        val tree = localTreeUri
        localError = null
        if (tree == null) {
            localEntries = emptyList()
            localLoading = false
            return@LaunchedEffect
        }
        if (tree != listedLocalRoot || localPath != listedLocalPath) localEntries = emptyList()
        localLoading = true
        repository.listSafDirectory(tree, localPath)
            .onSuccess { localEntries = it; listedLocalRoot = tree; listedLocalPath = localPath }
            .onFailure { localEntries = emptyList(); localError = describeSftpError(it) }
        localLoading = false
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
            navigateLocal("")
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
        localTreePicker.launch(localTreeUri?.takeUnless(::isSharedStorageTree)?.let(android.net.Uri::parse))
    }

    val requestLocalAccess = rememberAllFilesAccessRequest {
        backgroundPolicy?.endUserFlow()
        val availableRoot = repository.defaultLocalTreeUri()
        if (availableRoot != null) {
            localTreeUri = availableRoot
            navigateLocal("")
            val pending = pendingDownload
            pendingDownload = null
            if (pending != null) executeDownloads(pending)
        }
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
            pane == FsPane.Remote && remoteSearchVisible -> { remoteSearchVisible = false; remoteQuery = "" }
            pane == FsPane.Local && localSearchVisible -> { localSearchVisible = false; localQuery = "" }
            pane == FsPane.Remote && remoteSelecting -> { selectedRemote = emptyList(); remoteSelecting = false }
            pane == FsPane.Local && localSelecting -> { selectedLocal = emptyList(); localSelecting = false }
            pane == FsPane.Local && localPath.isNotBlank() -> {
                navigateLocal(parentLocalPath(localPath))
            }
            else -> onDismiss()
        }
    }

    val ready = session?.phase == SftpPhase.Ready
    val visibleRemoteEntries = remember(directory.entries, directory.loading, directory.path, remotePath,
        remoteQuery, remoteSort, remoteSortAscending) {
        // Refresh retains its files; a different directory must never show the previous one's actions.
        if (directory.loading && directory.path != remotePath) emptyList()
        else visibleFiles(directory.entries, remoteQuery, FileSort.valueOf(remoteSort), remoteSortAscending)
    }
    val visibleLocalEntries = remember(localEntries, localQuery, localSort, localSortAscending) {
        visibleLocalFiles(localEntries, localQuery, FileSort.valueOf(localSort), localSortAscending)
    }
    val twoPanes = LocalTwoPaneAvailable.current
    val localRootLabel = if (isSharedStorageTree(localTreeUri)) stringResource(R.string.sftp_storage_internal)
        else localTreeUri?.let(repository::safTreeLabel) ?: stringResource(R.string.fm_local)
    val activePath = if (pane == FsPane.Local) localPath else remotePath.ifBlank { directory.path }
    val folderTitle = activePath.trimEnd('/').substringAfterLast('/').takeIf { it.isNotBlank() && it != "." }
        ?: if (pane == FsPane.Local) localRootLabel
        else stringResource(R.string.sftp_browser_root)
    Scaffold(
        modifier = modifier,
        snackbarHost = { SnackbarHost(snackbar) },
        containerColor = MaterialTheme.colorScheme.surfaceContainer,
        bottomBar = {
            SftpBrowserFooter(local = pane == FsPane.Local, showPaneSwitch = ready && !twoPanes,
                onSelect = { local -> scope.launch { pagerState.animateScrollToPage(if (local) 0 else 1) } },
                panePosition = { pagerState.currentPage + pagerState.currentPageOffsetFraction },
                transfers = visibleTransfers, onTransfers = { transferSheetVisible = true })
        },
        floatingActionButton = {
            if (ready && (pane == FsPane.Remote || twoPanes) && !remoteSelecting && !localSelecting) SftpCreateAction(
                onUploadFiles = { backgroundPolicy?.beginUserFlow(); uploadFilePicker.launch(arrayOf("*/*")) },
                onUploadFolder = { requestLocalTree() },
                onCreateFolder = { newFolderName = ""; createFolderVisible = true }
            )
        }
    ) { contentPadding ->
        Column(modifier = Modifier.fillMaxSize().padding(contentPadding)) {
            SftpConnectionHeader(
                profile = profile,
                session = session,
                title = if (ready && !twoPanes) folderTitle else "SFTP",
                onMenu = onMenu,
                searchAvailable = ready && !twoPanes && (pane == FsPane.Remote || localTreeUri != null),
                onSearch = { if (pane == FsPane.Local) localSearchVisible = true else remoteSearchVisible = true },
                onTransfers = { transferSheetVisible = true },
                onRefresh = { if (pane == FsPane.Local) localRefresh += 1 else scope.launch { remoteListing() } },
                onCancel = {
                    if (onRequestClose != null && handle != null) onRequestClose(handle!!.connectionId)
                    else if (transfers.any { !it.state.isTerminal }) confirmCloseTransfers = true
                    else closeSftp()
                }
            )
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
                        searchVisible = localSearchVisible,
                        showSearchAction = twoPanes,
                        onOpenSearch = { localSearchVisible = true },
                        onCloseSearch = { localSearchVisible = false; localQuery = "" },
                        selecting = localSelecting,
                        onBeginSelection = { localSelecting = true },
                        query = localQuery, onQuery = { localQuery = it; localListState.requestScrollToItem(0) },
                        sort = FileSort.valueOf(localSort), sortAscending = localSortAscending,
                        onSort = {
                            localSortAscending = if (localSort == it.name) !localSortAscending else it == FileSort.Name
                            localSort = it.name
                            localListState.requestScrollToItem(0)
                        },
                        listState = localListState,
                        treeUri = localTreeUri,
                        treeLabel = localRootLabel,
                        path = localPath,
                        onNavigatePath = ::navigateLocal,
                        entries = visibleLocalEntries,
                        loading = localLoading,
                        error = localError,
                        selected = selectedLocal.toSet(),
                        onPickTree = { requestLocalTree() },
                        onRequestAccess = { backgroundPolicy?.beginUserFlow(); requestLocalAccess() },
                        onNavigateUp = {
                            navigateLocal(parentLocalPath(localPath))
                        },
                        onRefresh = { localRefresh += 1 },
                        onEnter = { entry ->
                            navigateLocal(entry.relativePath)
                        },
                        onToggleSelection = { entry ->
                            localSelecting = true
                            haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.TextHandleMove)
                            selectedLocal = (selectedLocal.toSet()).toggle(entry.relativePath).toList()
                        },
                        onSelectAll = {
                            selectedLocal = visibleLocalEntries.map { it.relativePath }
                        },
                        onClearSelection = { selectedLocal = emptyList(); localSelecting = false },
                        onUploadToRemote = { entry ->
                            val targets = entry?.let { listOf(it) } ?: selectedLocalEntries()
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
                        searchVisible = remoteSearchVisible,
                        showSearchAction = twoPanes,
                        onOpenSearch = { remoteSearchVisible = true },
                        onCloseSearch = { remoteSearchVisible = false; remoteQuery = "" },
                        selecting = remoteSelecting,
                        onBeginSelection = { remoteSelecting = true },
                        query = remoteQuery, onQuery = { remoteQuery = it; remoteListState.requestScrollToItem(0) },
                        sort = FileSort.valueOf(remoteSort), sortAscending = remoteSortAscending,
                        onSort = {
                            remoteSortAscending = if (remoteSort == it.name) !remoteSortAscending else it == FileSort.Name
                            remoteSort = it.name
                            remoteListState.requestScrollToItem(0)
                        },
                        listState = remoteListState,
                        onNavigatePath = ::navigateRemote,
                        path = remotePath.ifBlank { directory.path },
                        entries = visibleRemoteEntries,
                        loading = directory.loading,
                        error = directory.error,
                        selected = selectedRemote.toSet(),
                        onNavigateUp = {
                            navigateRemote(parentRemotePath(remotePath))
                        },
                        onRefresh = { scope.launch { remoteListing() } },
                        onEnter = { entry ->
                            navigateRemote(entry.path)
                        },
                        onToggleSelection = { entry ->
                            remoteSelecting = true
                            haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.TextHandleMove)
                            selectedRemote = selectedRemote.toSet().toggle(entry.path).toList()
                        },
                        onSelectAll = {
                            selectedRemote = visibleRemoteEntries.map { it.path }
                        },
                        onClearSelection = { selectedRemote = emptyList(); remoteSelecting = false },
                        onEditPath = {
                            pathInput = remotePath
                            pathEditVisible = true
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
                        onDownload = { entries ->
                            if (twoPanes) paneName = FsPane.Local.name
                            else scope.launch { pagerState.animateScrollToPage(0) }
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
            if (twoPanes && ready) {
                Row(
                    Modifier.weight(1f).fillMaxWidth().padding(horizontal = UiSpacing.sm),
                    horizontalArrangement = Arrangement.spacedBy(UiSpacing.sm)
                ) {
                    SftpPaneSurface(Modifier.weight(1f), rounded = true) { paneContent(0) }
                    SftpPaneSurface(Modifier.weight(1f), rounded = true) { paneContent(1) }
                }
            } else HorizontalPager(
                state = pagerState,
                modifier = Modifier.weight(1f).fillMaxWidth().testTag("sftp-pane-pager"),
                pageSpacing = UiSpacing.xs,
                userScrollEnabled = ready
            ) { page -> SftpPaneSurface { paneContent(page) } }
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
                        navigateRemote(pathInput.trim().ifBlank { "." })
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
        ModalBottomSheet(onDismissRequest = { transferSheetVisible = false }, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
            Column(modifier = Modifier.fillMaxWidth().padding(horizontal = UiSpacing.sm).testTag("sftp-transfer-list")) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        stringResource(R.string.sftp_transfers_title),
                        style = MaterialTheme.typography.headlineSmall,
                        modifier = Modifier.weight(1f)
                    )
                    IconButton(onClick = { transferSheetVisible = false }) {
                        Icon(Icons.Outlined.Close, stringResource(android.R.string.cancel))
                    }
                }
                if (visibleTransfers.isEmpty()) SftpBrowserEmptyState(
                    icon = Icons.Outlined.CloudDownload,
                    title = stringResource(R.string.sftp_browser_transfer_empty),
                    description = stringResource(R.string.sftp_browser_transfer_empty_hint),
                    modifier = Modifier.fillMaxWidth().heightIn(min = 280.dp)
                ) else {
                    TextButton(onClick = { repository.transferActions.hideCompleted(sheetTransfers); recordsRevision += 1 }) {
                        Text(stringResource(R.string.ux_clear_completed))
                    }
                    LazyColumn(verticalArrangement = Arrangement.spacedBy(UiSpacing.xs), contentPadding = PaddingValues(bottom = UiSpacing.md)) {
                        items(visibleTransfers, key = { "${it.connectionId}:${it.transferId}" }) { transfer ->
                            SftpTransferRow(
                                transfer = transfer,
                                onRetry = { retryTarget = transfer },
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
}

@Composable
private fun SftpConnectionHeader(
    profile: HostProfileUi,
    session: SftpSessionModel?,
    title: String,
    searchAvailable: Boolean,
    onMenu: () -> Unit,
    onSearch: () -> Unit,
    onTransfers: () -> Unit,
    onRefresh: () -> Unit,
    onCancel: () -> Unit
) {
    val connected = session?.phase == SftpPhase.Ready
    var actionsVisible by remember { mutableStateOf(false) }
    val statusLabel = stringResource(if (connected) R.string.ux_connected else R.string.ux_waiting)
    Surface(color = MaterialTheme.colorScheme.surfaceContainer) {
        SessionTopBarCapsule {
            IconButton(onClick = onMenu) {
                Icon(Icons.Outlined.Menu, stringResource(R.string.links_sidebar_toggle),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Column(Modifier.weight(1f).padding(horizontal = UiSpacing.xs).semantics { stateDescription = statusLabel },
                verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(title, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
                    Box(Modifier.size(6.dp).background(
                        if (connected) successColor else MaterialTheme.colorScheme.onSurfaceVariant, CircleShape))
                    Text(profile.name + " · SFTP", style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            if (searchAvailable) IconButton(onClick = onSearch, modifier = Modifier.testTag("sftp-search-action")) {
                Icon(Icons.Outlined.Search, stringResource(R.string.ux_filter_directory),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Box {
                IconButton(onClick = { actionsVisible = true }) {
                    Icon(Icons.Outlined.MoreVert, stringResource(R.string.fm_more_actions),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                DropdownMenu(expanded = actionsVisible, onDismissRequest = { actionsVisible = false }) {
                    if (connected) DropdownMenuItem(text = { Text(stringResource(R.string.fm_refresh)) },
                        onClick = { actionsVisible = false; onRefresh() }, leadingIcon = { Icon(Icons.Outlined.Refresh, null) })
                    DropdownMenuItem(text = { Text(stringResource(R.string.sftp_transfers_title)) },
                        onClick = { actionsVisible = false; onTransfers() }, leadingIcon = { Icon(Icons.Outlined.SwapVert, null) })
                    if (connected) DropdownMenuItem(text = { Text(stringResource(R.string.terminal_close_session)) },
                        onClick = { actionsVisible = false; onCancel() }, leadingIcon = { Icon(Icons.Outlined.Close, null) })
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
        SftpBrowserEmptyState(
            icon = Icons.Outlined.FolderOff,
            title = stringResource(R.string.ui_connection_failed),
            description = error ?: session?.failure ?: session?.let(::phaseLabel).orEmpty(),
            modifier = Modifier.fillMaxSize(),
            action = { dev.akko.miaominal.ui.components.ConnectionErrorActions(
                error ?: session?.failure.orEmpty(), onRetry, onEdit) }
        )
    } else SftpBrowserLoadingState(
        status = session?.let(::phaseLabel) ?: uiString(R.string.ui_opening_sftp_session),
        modifier = Modifier.fillMaxSize()
    )
}

@OptIn(ExperimentalFoundationApi::class, ExperimentalMaterial3Api::class)
@Composable
private fun RemotePane(
    searchVisible: Boolean, showSearchAction: Boolean,
    onOpenSearch: () -> Unit, onCloseSearch: () -> Unit,
    selecting: Boolean, onBeginSelection: () -> Unit,
    query: String, onQuery: (String) -> Unit,
    sort: FileSort, sortAscending: Boolean, onSort: (FileSort) -> Unit,
    listState: LazyListState,
    onNavigatePath: (String) -> Unit,
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
    onRename: (SftpEntryModel) -> Unit,
    onDelete: () -> Unit,
    onDeleteEntry: (SftpEntryModel) -> Unit,
    onUploadFiles: () -> Unit,
    onDownload: (List<SftpEntryModel>) -> Unit
) {
    var detail by remember { mutableStateOf<SftpEntryModel?>(null) }
    val context = LocalContext.current
    detail?.let { entry ->
        ModalBottomSheet(onDismissRequest = { detail = null }) {
            Column(Modifier.fillMaxWidth().padding(24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                SftpFileDetailHeader(entry.name, entry.path, entryIcon(entry),
                    if (entry.isDirectory) uiString(R.string.ui_folder) else formatBytes(entry.size))
                FilledTonalButton(onClick = { detail = null; onDownload(listOf(entry)) }, modifier = Modifier.fillMaxWidth()) {
                    Icon(Icons.Outlined.Download, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(UiSpacing.xs))
                    Text(stringResource(R.string.fm_download))
                }
                TextButton(onClick = { detail = null; onRename(entry) }) { Text(stringResource(R.string.fm_rename)) }
                TextButton(onClick = { detail = null; onDeleteEntry(entry) }) { Text(stringResource(R.string.action_delete)) }
                TextButton(onClick = { (context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as ClipboardManager)
                    .setPrimaryClip(ClipData.newPlainText("path", entry.path)) }) { Text(stringResource(R.string.ux_copy_path)) }
            }
        }
    }
    Column(modifier = Modifier.fillMaxSize()) {
        SftpDirectoryBar(
            local = false, path = path.ifBlank { "." }, count = entries.size,
            onNavigateUp = onNavigateUp, onNavigate = onNavigatePath,
            onPathAction = onEditPath, onRefresh = onRefresh, refreshing = loading,
            searchVisible = searchVisible, showSearchAction = showSearchAction,
            onOpenSearch = onOpenSearch, onCloseSearch = onCloseSearch,
            query = query, onQuery = onQuery, sort = sort, sortAscending = sortAscending,
            onSort = onSort, onSelect = onBeginSelection
        )
        if (selecting || selected.isNotEmpty()) {
            SelectionBar(
                count = selected.size,
                onSelectAll = onSelectAll,
                onClear = onClearSelection,
                actionLabel = stringResource(R.string.fm_download),
                actionIcon = Icons.Outlined.Download,
                onAction = { onDownload(entries.filter { it.path in selected }) },
                extraActions = {
                    var selectionMenu by remember { mutableStateOf(false) }
                    Box {
                        IconButton(onClick = { selectionMenu = true }) { Icon(Icons.Outlined.MoreVert, stringResource(R.string.fm_more_actions)) }
                        DropdownMenu(selectionMenu, { selectionMenu = false }) {
                            DropdownMenuItem(text = { Text(stringResource(R.string.fm_delete_selected)) }, enabled = selected.isNotEmpty(), onClick = { selectionMenu = false; onDelete() })
                        }
                    }
                }
            )
        }
        SftpDirectoryContent(loading = loading, hasEntries = entries.isNotEmpty(), error = error,
            modifier = Modifier.weight(1f).fillMaxWidth()) {
            if (error != null) {
                SftpBrowserEmptyState(Icons.Outlined.FolderOff, stringResource(R.string.ui_could_not_open_directory),
                    description = error,
                    modifier = Modifier.fillMaxSize(),
                    action = { Button(onClick = onRefresh, enabled = !loading) { Text(stringResource(R.string.sftp_retry)) } })
            } else if (entries.isEmpty()) {
                SftpBrowserEmptyState(
                    if (query.isNotBlank()) Icons.Outlined.Search else Icons.Outlined.FolderOpen,
                    stringResource(if (query.isNotBlank()) R.string.ux_empty_filter else R.string.sftp_no_entries),
                    stringResource(if (query.isNotBlank()) R.string.sftp_browser_filter_empty_hint else R.string.sftp_browser_directory_empty_hint),
                    modifier = Modifier.fillMaxSize(),
                    action = {
                        if (query.isNotBlank()) TextButton(onClick = { onQuery("") }) { Text(stringResource(R.string.sftp_browser_clear_search)) }
                        else FilledTonalButton(onClick = onUploadFiles) { Text(stringResource(R.string.sftp_upload_files)) }
                    }
                )
            } else {
                LazyColumn(state = listState, modifier = Modifier.fillMaxSize().testTag("sftp-remote-files"), contentPadding = PaddingValues(top = UiSpacing.xxs, bottom = 88.dp)) {
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
    SftpFileRow(
        name = entry.name,
        metadata = listOfNotNull(
            if (entry.isDirectory) uiString(R.string.ui_folder) else formatBytes(entry.size),
            (entry.owner ?: entry.attributes)?.takeIf { it.isNotBlank() }
        ).joinToString(" · "),
        directory = entry.isDirectory, selected = selected, icon = entryIcon(entry),
        onClick = onClick, onLongClick = onLongClick,
        trailing = {
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
    searchVisible: Boolean, showSearchAction: Boolean,
    onOpenSearch: () -> Unit, onCloseSearch: () -> Unit,
    selecting: Boolean, onBeginSelection: () -> Unit,
    query: String, onQuery: (String) -> Unit,
    sort: FileSort, sortAscending: Boolean, onSort: (FileSort) -> Unit,
    listState: LazyListState,
    treeUri: String?,
    treeLabel: String?,
    path: String,
    onNavigatePath: (String) -> Unit,
    entries: List<SafLocalEntry>,
    loading: Boolean,
    error: String?,
    selected: Set<String>,
    onPickTree: () -> Unit,
    onRequestAccess: () -> Unit,
    onNavigateUp: () -> Unit,
    onRefresh: () -> Unit,
    onEnter: (SafLocalEntry) -> Unit,
    onToggleSelection: (SafLocalEntry) -> Unit,
    onSelectAll: () -> Unit,
    onClearSelection: () -> Unit,
    onUploadToRemote: (SafLocalEntry?) -> Unit
) {
    var detail by remember { mutableStateOf<SafLocalEntry?>(null) }
    val context = LocalContext.current
    detail?.let { entry -> ModalBottomSheet(onDismissRequest = { detail = null }) {
        Column(Modifier.fillMaxWidth().padding(UiSpacing.md), verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
            SftpFileDetailHeader(entry.name, entry.relativePath, sftpFileIcon(entry.name, entry.isDirectory),
                if (entry.isDirectory) uiString(R.string.ui_folder) else formatBytes(entry.size))
            FilledTonalButton(onClick = { detail = null; onUploadToRemote(entry) }, modifier = Modifier.fillMaxWidth()) {
                Icon(Icons.Outlined.Upload, null, Modifier.size(18.dp))
                Spacer(Modifier.width(UiSpacing.xs))
                Text(stringResource(R.string.fm_upload_to_remote))
            }
            TextButton(onClick = { detail = null; onToggleSelection(entry) }) { Text(stringResource(R.string.ux_choose)) }
            TextButton(onClick = { (context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as ClipboardManager)
                .setPrimaryClip(ClipData.newPlainText("path", entry.relativePath)) }) { Text(stringResource(R.string.ux_copy_path)) }
        }
    } }
    Column(modifier = Modifier.fillMaxSize()) {
        if (treeUri == null) {
            SftpBrowserEmptyState(
                icon = Icons.Outlined.FolderOpen,
                title = stringResource(R.string.sftp_browser_local_title),
                description = stringResource(R.string.sftp_storage_local_hint),
                modifier = Modifier.fillMaxSize(),
                action = { Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Button(onClick = onRequestAccess) { Text(stringResource(R.string.sftp_storage_allow)) }
                    TextButton(onClick = onPickTree) { Text(stringResource(R.string.sftp_local_pick)) }
                } }
            )
            return@Column
        }
        SftpDirectoryBar(
            local = true, path = path, rootLabel = treeLabel, count = entries.size,
            onNavigateUp = onNavigateUp, onNavigate = onNavigatePath,
            onPathAction = onPickTree, onRefresh = onRefresh, refreshing = loading,
            searchVisible = searchVisible, showSearchAction = showSearchAction,
            onOpenSearch = onOpenSearch, onCloseSearch = onCloseSearch,
            query = query, onQuery = onQuery, sort = sort, sortAscending = sortAscending,
            onSort = onSort, onSelect = onBeginSelection
        )
        if (selecting || selected.isNotEmpty()) {
            SelectionBar(
                count = selected.size,
                onSelectAll = onSelectAll,
                onClear = onClearSelection,
                actionLabel = stringResource(R.string.fm_upload_to_remote),
                actionIcon = Icons.Outlined.Upload,
                onAction = { onUploadToRemote(null) }
            )
        }
        SftpDirectoryContent(loading = loading, hasEntries = entries.isNotEmpty(), error = error,
            modifier = Modifier.weight(1f).fillMaxWidth()) {
            if (error != null) {
                SftpBrowserEmptyState(Icons.Outlined.FolderOff, stringResource(R.string.ui_could_not_open_directory),
                    description = error, modifier = Modifier.fillMaxSize(),
                    action = { Button(onClick = onRefresh, enabled = !loading) { Text(stringResource(R.string.sftp_retry)) } })
            } else if (entries.isEmpty()) {
                SftpBrowserEmptyState(
                    if (query.isNotBlank()) Icons.Outlined.Search else Icons.Outlined.FolderOpen,
                    stringResource(if (query.isNotBlank()) R.string.ux_empty_filter else R.string.fm_local_empty),
                    stringResource(if (query.isNotBlank()) R.string.sftp_browser_filter_empty_hint else R.string.sftp_browser_local_empty_hint),
                    modifier = Modifier.fillMaxSize(),
                    action = { if (query.isNotBlank()) TextButton(onClick = { onQuery("") }) { Text(stringResource(R.string.sftp_browser_clear_search)) } }
                )
            } else {
                LazyColumn(state = listState, modifier = Modifier.fillMaxSize().testTag("sftp-local-files"), contentPadding = PaddingValues(top = UiSpacing.xxs, bottom = UiSpacing.sm)) {
                    items(entries, key = { it.relativePath }) { entry ->
                        SftpFileRow(
                            name = entry.name,
                            metadata = if (entry.isDirectory) uiString(R.string.ui_folder) else formatBytes(entry.size),
                            directory = entry.isDirectory, selected = entry.relativePath in selected,
                            onClick = { if (selecting || selected.isNotEmpty()) onToggleSelection(entry)
                                    else if (entry.isDirectory) onEnter(entry) else detail = entry },
                            onLongClick = { onToggleSelection(entry) },
                            trailing = {
                                if (entry.isDirectory && entry.relativePath !in selected) Icon(Icons.Outlined.ChevronRight, null,
                                    Modifier.padding(end = UiSpacing.sm), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun SelectionBar(
    count: Int,
    onSelectAll: () -> Unit,
    onClear: () -> Unit,
    actionLabel: String,
    actionIcon: ImageVector,
    onAction: () -> Unit,
    extraActions: @Composable () -> Unit = {}
) {
    val colors = MaterialTheme.colorScheme
    Surface(
        modifier = Modifier.fillMaxWidth().padding(horizontal = UiSpacing.xs, vertical = UiSpacing.xxs)
            .testTag("sftp-selection-bar"),
        shape = MaterialTheme.shapes.extraLarge,
        color = colors.secondaryContainer,
        contentColor = colors.onSecondaryContainer
    ) {
        Column(Modifier.padding(start = UiSpacing.sm, end = UiSpacing.xs, bottom = UiSpacing.xs)) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Outlined.CheckCircle, null, Modifier.size(20.dp))
                Spacer(Modifier.width(UiSpacing.xs))
                Text(
                    text = stringResource(R.string.fm_selected_count, count),
                    style = MaterialTheme.typography.titleSmall,
                    maxLines = 1, overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f)
                )
                extraActions()
                IconButton(onClick = onClear, modifier = Modifier.testTag("sftp-selection-close")) {
                    Icon(Icons.Outlined.Close, contentDescription = stringResource(R.string.fm_clear_selection))
                }
            }
            val selectAllLabel = stringResource(R.string.fm_select_all)
            val labelStyle = MaterialTheme.typography.labelLarge
            val measurer = rememberTextMeasurer()
            val density = LocalDensity.current
            val iconSize = with(density) { maxOf(18.dp, labelStyle.fontSize.toDp()) }
            val labelWidth = with(density) {
                maxOf(measurer.measure(selectAllLabel, labelStyle, maxLines = 1).size.width,
                    measurer.measure(actionLabel, labelStyle, maxLines = 1).size.width).toDp()
            }
            val minButtonWidth = labelWidth + iconSize + UiSpacing.xs * 3
            BoxWithConstraints(Modifier.fillMaxWidth().padding(end = UiSpacing.xs)) {
                if (maxWidth < minButtonWidth * 2 + UiSpacing.xs) {
                    Column(verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                        SelectionActionButton(selectAllLabel, Icons.Outlined.SelectAll, iconSize, onSelectAll,
                            Modifier.fillMaxWidth(), primary = false)
                        SelectionActionButton(actionLabel, actionIcon, iconSize, onAction,
                            Modifier.fillMaxWidth().testTag("sftp-selection-action"), enabled = count > 0)
                    }
                } else {
                    Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min),
                        horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                        SelectionActionButton(selectAllLabel, Icons.Outlined.SelectAll, iconSize, onSelectAll,
                            Modifier.weight(1f).fillMaxHeight(), primary = false)
                        SelectionActionButton(actionLabel, actionIcon, iconSize, onAction,
                            Modifier.weight(1f).fillMaxHeight().testTag("sftp-selection-action"), enabled = count > 0)
                    }
                }
            }
        }
    }
}

@Composable
private fun SelectionActionButton(label: String, icon: ImageVector, iconSize: androidx.compose.ui.unit.Dp,
    onClick: () -> Unit, modifier: Modifier, enabled: Boolean = true, primary: Boolean = true) {
    val content: @Composable RowScope.() -> Unit = {
        Icon(icon, null, Modifier.size(iconSize))
        Spacer(Modifier.width(UiSpacing.xs))
        Text(label, style = MaterialTheme.typography.labelLarge, textAlign = TextAlign.Center, maxLines = 1)
    }
    val padding = PaddingValues(horizontal = UiSpacing.xs, vertical = UiSpacing.xs)
    if (primary) Button(onClick = onClick, enabled = enabled, modifier = modifier.heightIn(min = 48.dp),
        contentPadding = padding, content = content)
    else OutlinedButton(onClick = onClick, enabled = enabled, modifier = modifier.heightIn(min = 48.dp),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline), contentPadding = padding,
        colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.onSecondaryContainer),
        content = content)
}

@Composable
private fun SftpTransferRow(
    transfer: SftpTransferModel,
    onRetry: () -> Unit,
    onPause: () -> Unit,
    onResume: () -> Unit,
    onCancel: () -> Unit
) {
    Surface(shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainerLowest) {
        Column(modifier = Modifier.fillMaxWidth().padding(UiSpacing.sm), verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Surface(shape = CircleShape, color = MaterialTheme.colorScheme.primaryContainer, modifier = Modifier.size(40.dp)) {
                    Box(contentAlignment = Alignment.Center) { Icon(transferIcon(transfer), contentDescription = null, tint = MaterialTheme.colorScheme.onPrimaryContainer) }
                }
                Spacer(Modifier.width(8.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        transfer.sourceLabel.ifBlank { transfer.destination },
                        style = MaterialTheme.typography.titleSmall,
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
                progress = { transfer.progress.coerceIn(0f, 1f) },
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
        }
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
    else -> sftpFileIcon(entry.name, entry.isDirectory)
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
