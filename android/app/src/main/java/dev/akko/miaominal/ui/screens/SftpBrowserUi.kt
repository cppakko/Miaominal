package dev.akko.miaominal.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.InsertDriveFile
import androidx.compose.material.icons.automirrored.outlined.Sort
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.MotionDurationScale
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.changedToDown
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.components.SessionStatusMessage
import dev.akko.miaominal.ui.model.FileSort
import dev.akko.miaominal.ui.theme.UiSpacing
import dev.akko.miaominal.ui.theme.MiaominalMotion
import kotlinx.coroutines.currentCoroutineContext

/** Presentation shared by the local and remote browsers. All file actions stay in SftpScreen. */
@Composable
internal fun SftpPaneSurface(modifier: Modifier = Modifier, rounded: Boolean = false,
    onActivate: (() -> Unit)? = null, content: @Composable () -> Unit) {
    val activate by rememberUpdatedState(onActivate)
    val paneModifier = if (onActivate == null) modifier else modifier.pointerInput(Unit) {
        awaitPointerEventScope {
            while (true) {
                val event = awaitPointerEvent(PointerEventPass.Initial)
                if (event.changes.any { it.changedToDown() }) activate?.invoke()
            }
        }
    }
    Surface(
        modifier = paneModifier.fillMaxSize(),
        color = if (rounded) MaterialTheme.colorScheme.surfaceContainerLow else MaterialTheme.colorScheme.surfaceContainer,
        shape = if (rounded) MaterialTheme.shapes.extraLarge else RectangleShape,
        content = content
    )
}

@Composable
internal fun SftpTransferBadge(count: Int, modifier: Modifier = Modifier) {
    if (count > 0) Box(modifier.background(MaterialTheme.colorScheme.primaryContainer, CircleShape)
        .padding(2.dp).clearAndSetSemantics {}) {
        Badge { Text(count.toString()) }
    }
}

@Composable
internal fun SftpSelectionToolbar(local: Boolean, count: Int, transferCount: Int,
    onCut: () -> Unit, onCopy: () -> Unit, onDelete: () -> Unit, onRename: () -> Unit,
    onCopyPaths: () -> Unit, onTransfer: () -> Unit, onTransfers: () -> Unit, modifier: Modifier = Modifier) {
    var moreVisible by remember { mutableStateOf(false) }
    Surface(modifier.testTag("sftp-selection-toolbar"), shape = CircleShape,
        color = MaterialTheme.colorScheme.primaryContainer, contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
        shadowElevation = 3.dp) {
        BoxWithConstraints {
            Row(Modifier.horizontalScroll(rememberScrollState()).widthIn(min = maxWidth)
                .heightIn(min = 64.dp).padding(horizontal = UiSpacing.xs), verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceEvenly) {
                if (!local) {
                    IconButton(onClick = onCut, enabled = count > 0, modifier = Modifier.testTag("sftp-selection-cut")) {
                        Icon(Icons.Outlined.ContentCut, stringResource(R.string.sftp_browser_cut))
                    }
                }
                IconButton(onClick = if (local) onCopyPaths else onCopy, enabled = count > 0,
                    modifier = Modifier.testTag("sftp-selection-copy")) {
                    Icon(Icons.Outlined.ContentCopy, stringResource(if (local) R.string.ux_copy_path else R.string.sftp_browser_copy))
                }
                if (!local) IconButton(onClick = onDelete, enabled = count > 0, modifier = Modifier.testTag("sftp-selection-delete")) {
                    Icon(Icons.Outlined.Delete, stringResource(R.string.fm_delete_selected))
                }
                IconButton(onClick = onTransfer, enabled = count > 0, modifier = Modifier.testTag("sftp-selection-action")) {
                    Icon(if (local) Icons.Outlined.Upload else Icons.Outlined.Download,
                        stringResource(if (local) R.string.fm_upload_to_remote else R.string.fm_download))
                }
                if (!local) IconButton(onClick = onRename, enabled = count == 1, modifier = Modifier.testTag("sftp-selection-rename")) {
                    Icon(Icons.Outlined.Edit, stringResource(R.string.fm_rename))
                }
                Box {
                    IconButton(onClick = { moreVisible = true }, modifier = Modifier.testTag("sftp-selection-more")) {
                        Icon(Icons.Outlined.MoreVert, stringResource(R.string.fm_more_actions))
                    }
                    SftpTransferBadge(transferCount, Modifier.align(Alignment.TopEnd).offset(x = (-2).dp, y = 2.dp))
                    DropdownMenu(moreVisible, { moreVisible = false }, shape = MaterialTheme.shapes.large) {
                        DropdownMenuItem(text = { Text(stringResource(R.string.ux_copy_path)) }, enabled = count > 0,
                            onClick = { moreVisible = false; onCopyPaths() }, leadingIcon = { Icon(Icons.Outlined.ContentCopy, null) })
                        if (transferCount > 0) DropdownMenuItem(text = { Text(stringResource(R.string.sftp_transfers_title)) },
                            modifier = Modifier.testTag("sftp-transfer-dock"),
                            onClick = { moreVisible = false; onTransfers() }, leadingIcon = { Icon(Icons.Outlined.SwapVert, null) })
                    }
                }
            }
        }
    }
}

@Composable
internal fun SftpPasteToolbar(local: Boolean, count: Int, busy: Boolean,
    onPaste: () -> Unit, onClear: () -> Unit, transferCount: Int, onTransfers: () -> Unit, modifier: Modifier = Modifier) {
    Surface(modifier.testTag("sftp-paste-toolbar"), shape = CircleShape,
        color = MaterialTheme.colorScheme.primaryContainer, contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
        shadowElevation = 3.dp) {
        Row(Modifier.heightIn(min = 64.dp).padding(horizontal = UiSpacing.xs), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = onPaste, enabled = !busy,
                modifier = Modifier.weight(1f).heightIn(min = 48.dp).testTag("sftp-paste-action"),
                colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.onPrimaryContainer)) {
                if (busy) CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
                else Icon(if (local) Icons.Outlined.Download else Icons.Outlined.ContentPaste, null)
                Spacer(Modifier.width(UiSpacing.xs))
                Text(stringResource(if (local) R.string.sftp_browser_download_here else R.string.sftp_browser_paste_here, count),
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            IconButton(onClick = onClear, enabled = !busy, modifier = Modifier.testTag("sftp-paste-clear")) {
                Icon(Icons.Outlined.Close, stringResource(R.string.sftp_browser_clear_clipboard))
            }
            if (transferCount > 0) Box {
                IconButton(onClick = onTransfers, modifier = Modifier.testTag("sftp-transfer-dock")) {
                    Icon(Icons.Outlined.SwapVert, stringResource(R.string.sftp_transfers_title))
                }
                SftpTransferBadge(transferCount, Modifier.align(Alignment.TopEnd).offset(x = (-2).dp, y = 2.dp))
            }
        }
    }
}

@Composable
internal fun SftpBrowserLoadingState(status: String, modifier: Modifier = Modifier) {
    BoxWithConstraints(modifier.testTag("sftp-browser-loading")) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .heightIn(min = maxHeight).padding(UiSpacing.md),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center
        ) {
            SessionStatusMessage(status = status, busy = true,
                modifier = Modifier.widthIn(max = 400.dp).fillMaxWidth())
        }
    }
}

private enum class DirectoryLoading { None, Centered, Refresh }
private enum class DirectoryContent { Pending, Loading, Ready }

/** Keep short requests quiet and let a visible indicator finish its entrance before leaving. */
@Composable
internal fun SftpDirectoryContent(loading: Boolean, hasEntries: Boolean, error: String?,
    modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    var indicator by remember { mutableStateOf(DirectoryLoading.None) }
    var shownAt by remember { mutableLongStateOf(0L) }
    val latestHasEntries by rememberUpdatedState(hasEntries)
    val failed = error != null
    LaunchedEffect(loading, failed) {
        val durationScale = currentCoroutineContext()[MotionDurationScale]?.scaleFactor ?: 1f
        when {
            failed -> indicator = DirectoryLoading.None
            durationScale == 0f -> indicator = when {
                !loading -> DirectoryLoading.None
                latestHasEntries -> DirectoryLoading.Refresh
                else -> DirectoryLoading.Centered
            }
            loading && indicator == DirectoryLoading.None -> {
                val startedAt = withFrameMillis { it }
                shownAt = awaitDirectoryLoadingFrame(startedAt + (150 * durationScale).toLong())
                indicator = if (latestHasEntries) DirectoryLoading.Refresh else DirectoryLoading.Centered
            }
            !loading && indicator != DirectoryLoading.None -> {
                awaitDirectoryLoadingFrame(shownAt + (MiaominalMotion.DurationLong1 * durationScale).toLong())
                indicator = DirectoryLoading.None
            }
        }
    }
    val state = when {
        failed -> DirectoryContent.Ready
        indicator == DirectoryLoading.Centered -> DirectoryContent.Loading
        loading && !hasEntries -> DirectoryContent.Pending
        else -> DirectoryContent.Ready
    }
    Column(modifier) {
        SftpRefreshProgress(visible = !failed && indicator == DirectoryLoading.Refresh)
        AnimatedContent(targetState = state, modifier = Modifier.weight(1f).fillMaxWidth(),
            transitionSpec = {
                (fadeIn(tween(MiaominalMotion.DurationMedium2, delayMillis = MiaominalMotion.DurationShort2,
                    easing = MiaominalMotion.StandardDecelerate)) togetherWith
                    fadeOut(tween(MiaominalMotion.DurationShort2, easing = MiaominalMotion.StandardAccelerate))).using(null)
            }, label = "sftp-directory-content") { displayed ->
            when (displayed) {
                DirectoryContent.Pending -> Box(Modifier.fillMaxSize())
                DirectoryContent.Loading -> SftpBrowserLoadingState(
                    stringResource(R.string.sftp_browser_loading_directory), Modifier.fillMaxSize())
                DirectoryContent.Ready -> Box(Modifier.fillMaxSize()) {
                    // An outgoing ready state must not announce an empty folder while the next one is loading.
                    if (!loading || hasEntries || failed) content()
                }
            }
        }
    }
}

@Composable
private fun SftpRefreshProgress(visible: Boolean) {
    // Reserve the strip so refresh never moves the file rows or their touch targets.
    Box(Modifier.fillMaxWidth().height(12.dp).padding(horizontal = UiSpacing.xs, vertical = UiSpacing.xxs)) {
        AnimatedVisibility(visible = visible,
            enter = fadeIn(MiaominalMotion.chromeFadeIn()), exit = fadeOut(MiaominalMotion.chromeFadeOut())) {
            LinearProgressIndicator(Modifier.fillMaxWidth().testTag("sftp-directory-refresh-progress"))
        }
    }
}

private suspend fun awaitDirectoryLoadingFrame(deadline: Long): Long {
    var now = withFrameMillis { it }
    while (now < deadline) now = withFrameMillis { it }
    return now
}

@Composable
internal fun SftpDirectoryBar(
    local: Boolean,
    path: String,
    rootLabel: String? = null,
    count: Int,
    onNavigateUp: () -> Unit,
    onNavigate: (String) -> Unit,
    onPathAction: () -> Unit,
    refreshing: Boolean = false,
    searchVisible: Boolean,
    showSearchAction: Boolean,
    onOpenSearch: () -> Unit,
    onCloseSearch: () -> Unit,
    query: String,
    onQuery: (String) -> Unit
) {
    val breadcrumbScroll = rememberScrollState()
    LaunchedEffect(path, breadcrumbScroll.maxValue) { breadcrumbScroll.animateScrollTo(breadcrumbScroll.maxValue) }
    Column(Modifier.fillMaxWidth()) {
        Surface(
            modifier = Modifier.fillMaxWidth().padding(horizontal = UiSpacing.xs),
            shape = RectangleShape,
            color = Color.Transparent,
            contentColor = MaterialTheme.colorScheme.onSurfaceVariant
        ) {
            AnimatedContent(targetState = searchVisible, modifier = Modifier.fillMaxWidth().testTag("sftp-search-transition"),
                transitionSpec = {
                    (fadeIn(tween(MiaominalMotion.DurationShort4, easing = MiaominalMotion.StandardDecelerate)) +
                        slideInVertically(tween(MiaominalMotion.DurationShort4)) { it / 6 }) togetherWith
                        (fadeOut(tween(MiaominalMotion.DurationShort2)) +
                            slideOutVertically(tween(MiaominalMotion.DurationShort2)) { -it / 6 })
                }, label = "sftp-directory-search") { searching ->
                if (searching) SftpSearchField(query, onQuery, onCloseSearch) else Row(
                    Modifier.fillMaxWidth().heightIn(min = 48.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    IconButton(onClick = onNavigateUp, enabled = if (local) path.isNotBlank() else path != "/") {
                        Icon(Icons.Outlined.ArrowUpward, stringResource(R.string.sftp_navigate_up))
                    }
                    Row(Modifier.weight(1f).horizontalScroll(breadcrumbScroll), verticalAlignment = Alignment.CenterVertically) {
                        val absolute = path.startsWith('/')
                        val segments = path.split('/').filter { it.isNotEmpty() && it != "." }
                        TextButton(
                            onClick = { onNavigate(if (local) "" else if (absolute) "/" else ".") },
                            contentPadding = PaddingValues(horizontal = UiSpacing.xs),
                            colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.onSurfaceVariant)
                        ) {
                            Icon(if (local) Icons.Outlined.PhoneAndroid else Icons.Outlined.Dns, null, Modifier.size(18.dp))
                            Spacer(Modifier.width(UiSpacing.xs))
                            Text(if (local) rootLabel ?: stringResource(R.string.fm_local) else if (absolute) "/" else ".", maxLines = 1)
                        }
                        segments.forEachIndexed { index, segment ->
                            Icon(Icons.Outlined.ChevronRight, null, Modifier.size(16.dp))
                            TextButton(
                                onClick = { onNavigate((if (absolute) "/" else "") + segments.take(index + 1).joinToString("/")) },
                                contentPadding = PaddingValues(horizontal = UiSpacing.xxs),
                                colors = ButtonDefaults.textButtonColors(contentColor =
                                    if (index == segments.lastIndex) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant)
                            ) { Text(segment, maxLines = 1) }
                        }
                    }
                    IconButton(onClick = onPathAction) {
                        Icon(if (local) Icons.Outlined.FolderOpen else Icons.Outlined.Edit,
                            stringResource(if (local) R.string.sftp_local_pick else R.string.sftp_edit_path))
                    }
                }
            }
        }
        if (showSearchAction) {
            Row(Modifier.fillMaxWidth().padding(horizontal = UiSpacing.xs, vertical = UiSpacing.xxs),
                verticalAlignment = Alignment.CenterVertically) {
                Text(if (refreshing && count == 0) stringResource(R.string.sftp_browser_loading_short)
                    else stringResource(R.string.sftp_browser_item_count, count),
                    Modifier.weight(1f).padding(start = UiSpacing.xs),
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
                IconButton(onClick = onOpenSearch) {
                    Icon(Icons.Outlined.Search, stringResource(R.string.ux_filter_directory))
                }
            }
        } else {
            Text(if (refreshing && count == 0) stringResource(R.string.sftp_browser_loading_short)
                else stringResource(R.string.sftp_browser_item_count, count),
                Modifier.fillMaxWidth().padding(horizontal = UiSpacing.sm, vertical = UiSpacing.xxs),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun SftpSearchField(query: String, onQuery: (String) -> Unit, onClose: () -> Unit) {
    val searchLabel = stringResource(R.string.ux_filter_directory)
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    TextField(
        value = query, onValueChange = onQuery,
        modifier = Modifier.fillMaxWidth().focusRequester(focus).testTag("sftp-directory-search").semantics { contentDescription = searchLabel },
        singleLine = true, shape = CircleShape,
        placeholder = { Text(searchLabel, style = MaterialTheme.typography.bodyMedium) },
        leadingIcon = { IconButton(onClick = onClose) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.sftp_browser_close_search)) } },
        trailingIcon = if (query.isNotEmpty()) ({
            IconButton(onClick = { onQuery("") }) { Icon(Icons.Outlined.Close, stringResource(R.string.sftp_browser_clear_search)) }
        }) else null,
        colors = TextFieldDefaults.colors(
            focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerLowest,
            unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerLowest,
            focusedTextColor = MaterialTheme.colorScheme.onSurface,
            unfocusedTextColor = MaterialTheme.colorScheme.onSurface,
            focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent
        )
    )
}

@Composable
internal fun SftpDirectoryOptionsMenu(expanded: Boolean, onDismissRequest: () -> Unit,
    sort: FileSort, ascending: Boolean, onSort: (FileSort) -> Unit, onBack: () -> Unit) {
    val options = listOf(FileSort.Name, FileSort.Modified, FileSort.Size)
    val captions = options.map { stringResource(sortCaption(it)) }
    val captionStyle = MaterialTheme.typography.labelSmall
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    val preferredWidths = with(density) {
        captions.map { caption ->
            (measurer.measure(caption, captionStyle, maxLines = 1).size.width.toDp() +
                captionStyle.fontSize.toDp() + UiSpacing.xxs * 3).coerceAtLeast(64.dp)
        }
    }
    val availableWidth = (LocalConfiguration.current.screenWidthDp.dp - UiSpacing.sm).coerceAtLeast(0.dp)
    val menuWidth = maxOf(280.dp, preferredWidths.reduce { a, b -> a + b } + UiSpacing.xs * 3)
        .coerceAtMost(availableWidth)
    DropdownMenu(expanded = expanded, onDismissRequest = onDismissRequest,
        modifier = Modifier.width(menuWidth).testTag("sftp-directory-options-menu"),
        shape = MaterialTheme.shapes.large,
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        shadowElevation = 3.dp) {
        DropdownMenuItem(text = { Text(stringResource(R.string.sftp_browser_sort_by)) },
            modifier = Modifier.testTag("sftp-options-back"), onClick = onBack,
            leadingIcon = { Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.action_back)) })
        Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min).padding(horizontal = UiSpacing.xs).selectableGroup(),
            horizontalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
            options.forEachIndexed { index, option ->
                val widthWeight = preferredWidths[index].value
                SftpSortOption(option, sort == option, ascending, { onSort(option) }, Modifier.weight(widthWeight).fillMaxHeight())
            }
        }
    }
}

@Composable
private fun SftpSortOption(option: FileSort, active: Boolean, ascending: Boolean, onClick: () -> Unit, modifier: Modifier) {
    val direction = stringResource(if (ascending) R.string.sftp_browser_sort_ascending else R.string.sftp_browser_sort_descending)
    val colors = MaterialTheme.colorScheme
    val tint = if (active) colors.onPrimaryContainer else colors.onSurfaceVariant
    val captionStyle = MaterialTheme.typography.labelSmall
    val directionIconSize = with(LocalDensity.current) { captionStyle.fontSize.toDp() }
    Column(modifier.clip(MaterialTheme.shapes.medium)
        .background(if (active) colors.primaryContainer else Color.Transparent)
        .selectable(selected = active, onClick = onClick, role = Role.RadioButton)
        .testTag("sftp-sort-${option.name.lowercase()}").semantics { if (active) stateDescription = direction }
        .heightIn(min = 80.dp).padding(horizontal = UiSpacing.xxs, vertical = UiSpacing.xs),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(UiSpacing.xxs, Alignment.CenterVertically)) {
        Box(Modifier.height(28.dp), contentAlignment = Alignment.Center) {
            Icon(when (option) {
                FileSort.Name -> Icons.Outlined.SortByAlpha
                FileSort.Modified -> Icons.Outlined.CalendarMonth
                FileSort.Size -> Icons.AutoMirrored.Outlined.Sort
            }, null, Modifier.size(24.dp), tint)
        }
        Row(verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(UiSpacing.xxs, Alignment.CenterHorizontally)) {
            Text(stringResource(sortCaption(option)), Modifier.weight(1f, fill = false), style = captionStyle,
                color = tint, maxLines = 1, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center)
            if (active) {
                Icon(if (ascending) Icons.Outlined.ArrowUpward else Icons.Outlined.ArrowDownward,
                    null, Modifier.size(directionIconSize), tint)
            }
        }
    }
}

private fun sortCaption(sort: FileSort): Int = when (sort) {
    FileSort.Name -> R.string.ux_sort_name
    FileSort.Size -> R.string.sftp_browser_sort_size
    FileSort.Modified -> R.string.sftp_browser_sort_modified
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun SftpFileRow(
    name: String,
    metadata: String,
    directory: Boolean,
    selected: Boolean,
    icon: ImageVector = sftpFileIcon(name, directory),
    onClick: () -> Unit,
    onLongClick: () -> Unit,
    trailing: @Composable () -> Unit = {}
) {
    val colors = MaterialTheme.colorScheme
    val selectedLabel = stringResource(R.string.ux_selected)
    Surface(
        modifier = Modifier.fillMaxWidth().padding(horizontal = UiSpacing.xs, vertical = UiSpacing.xxs),
        shape = MaterialTheme.shapes.large,
        color = if (selected) colors.secondaryContainer else colors.surfaceContainerLowest
    ) {
        Row(
            Modifier.combinedClickable(
                onClick = onClick, onLongClick = onLongClick,
                onLongClickLabel = stringResource(R.string.ux_selection)
            ).semantics { this.selected = selected }
                .heightIn(min = 68.dp).padding(start = UiSpacing.xs, end = UiSpacing.xxs, top = UiSpacing.xs, bottom = UiSpacing.xs),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(UiSpacing.sm)
        ) {
            Surface(
                modifier = Modifier.size(40.dp),
                shape = CircleShape,
                color = when { selected -> colors.primary; directory -> colors.primaryContainer; else -> colors.surfaceContainerHigh },
                contentColor = when { selected -> colors.onPrimary; directory -> colors.onPrimaryContainer; else -> colors.onSurfaceVariant }
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(if (selected) Icons.Outlined.Check else icon, if (selected) selectedLabel else null, Modifier.size(22.dp))
                }
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
                Text(name, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(metadata, style = MaterialTheme.typography.bodySmall, color = if (selected) colors.onSecondaryContainer else colors.onSurfaceVariant,
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            trailing()
        }
    }
}

internal fun sftpFileIcon(name: String, directory: Boolean): ImageVector {
    if (directory) return Icons.Outlined.Folder
    return when (name.substringAfterLast('.', "").lowercase(java.util.Locale.ROOT)) {
        "zip", "gz", "tar", "xz", "7z", "rar" -> Icons.Outlined.FolderZip
        "png", "jpg", "jpeg", "webp", "svg", "gif" -> Icons.Outlined.Image
        "json", "yaml", "yml", "toml", "xml", "sh", "py", "js", "rs", "kt" -> Icons.Outlined.Code
        "md", "txt", "log", "pdf" -> Icons.Outlined.Description
        else -> Icons.AutoMirrored.Outlined.InsertDriveFile
    }
}

@Composable
internal fun SftpFileDetailHeader(name: String, path: String, icon: ImageVector, metadata: String) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(UiSpacing.sm)) {
        Surface(shape = CircleShape, color = MaterialTheme.colorScheme.primaryContainer, modifier = Modifier.size(56.dp)) {
            Box(contentAlignment = Alignment.Center) { Icon(icon, null, Modifier.size(28.dp), MaterialTheme.colorScheme.onPrimaryContainer) }
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
            Text(name, style = MaterialTheme.typography.titleLarge)
            Text(metadata, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
    Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = MaterialTheme.shapes.large, modifier = Modifier.fillMaxWidth()) {
        SelectionContainer {
            Text(path, modifier = Modifier.padding(UiSpacing.sm), style = MaterialTheme.typography.bodySmall,
                fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace)
        }
    }
}

@Composable
internal fun SftpBrowserEmptyState(
    icon: ImageVector,
    title: String,
    description: String,
    modifier: Modifier = Modifier,
    action: @Composable () -> Unit = {}
) {
    Box(modifier, contentAlignment = Alignment.Center) {
        Column(
            Modifier.widthIn(max = 360.dp).verticalScroll(rememberScrollState()).padding(UiSpacing.md),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(UiSpacing.sm)
        ) {
            Surface(shape = CircleShape, color = MaterialTheme.colorScheme.secondaryContainer, modifier = Modifier.size(80.dp)) {
                Box(contentAlignment = Alignment.Center) { Icon(icon, null, Modifier.size(36.dp), MaterialTheme.colorScheme.onSecondaryContainer) }
            }
            Text(title, style = MaterialTheme.typography.titleMedium, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            Text(description, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            action()
        }
    }
}
