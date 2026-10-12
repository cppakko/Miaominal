package dev.akko.miaominal.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
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
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.Constraints
import dev.akko.miaominal.R
import dev.akko.miaominal.data.SftpTransferModel
import dev.akko.miaominal.data.SftpTransferStateModel
import dev.akko.miaominal.ui.components.SessionStatusMessage
import dev.akko.miaominal.ui.model.FileSort
import dev.akko.miaominal.ui.theme.UiSpacing
import dev.akko.miaominal.ui.theme.MiaominalMotion
import kotlinx.coroutines.currentCoroutineContext
import kotlin.math.roundToInt

/** Presentation shared by the local and remote browsers. All file actions stay in SftpScreen. */
@Composable
internal fun SftpPaneSwitch(local: Boolean, onSelect: (Boolean) -> Unit, panePosition: () -> Float,
    modifier: Modifier = Modifier) {
    val labels = listOf(stringResource(R.string.fm_local), stringResource(R.string.fm_remote))
    val labelStyle = MaterialTheme.typography.labelLarge
    val measurer = rememberTextMeasurer()
    val labelWidth = with(LocalDensity.current) {
        labels.maxOf { measurer.measure(it, labelStyle, maxLines = 1).size.width }.toDp()
    }
    BoxWithConstraints(modifier.selectableGroup()) {
        val stackLabels = labelWidth + 20.dp + UiSpacing.xs * 3 > (maxWidth - UiSpacing.xxs) / 2
        Layout(
            modifier = Modifier.matchParentSize(),
            content = {
                Box(Modifier.testTag("sftp-pane-indicator")
                    .background(MaterialTheme.colorScheme.secondaryContainer, CircleShape))
            }
        ) { measurables, constraints ->
            val gap = UiSpacing.xxs.roundToPx()
            val tabWidth = (constraints.maxWidth - gap) / 2
            val indicator = measurables.single().measure(Constraints.fixed(tabWidth, constraints.maxHeight))
            layout(constraints.maxWidth, constraints.maxHeight) {
                // Read the same progress as the pager during placement, without adding another animation.
                indicator.placeRelative(((tabWidth + gap) * panePosition().coerceIn(0f, 1f)).roundToInt(), 0)
            }
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
            listOf(true, false).forEach { isLocal ->
                val position = panePosition().coerceIn(0f, 1f)
                Surface(
                    onClick = { onSelect(isLocal) },
                    modifier = Modifier.weight(1f).testTag(if (isLocal) "sftp-pane-local" else "sftp-pane-remote")
                        .semantics { selected = local == isLocal; role = Role.Tab },
                    shape = CircleShape,
                    color = Color.Transparent,
                    contentColor = lerp(MaterialTheme.colorScheme.onSurfaceVariant,
                        MaterialTheme.colorScheme.onSecondaryContainer, if (isLocal) 1f - position else position)
                ) {
                    val label = labels[if (isLocal) 0 else 1]
                    val icon = if (isLocal) Icons.Outlined.PhoneAndroid else Icons.Outlined.Dns
                    if (stackLabels) Column(
                        Modifier.heightIn(min = 48.dp).padding(UiSpacing.xxs),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(UiSpacing.xxs, Alignment.CenterVertically)) {
                        Icon(icon, null, Modifier.size(20.dp))
                        Text(label, style = labelStyle, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    } else Row(Modifier.heightIn(min = 48.dp).padding(horizontal = UiSpacing.xs),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs, Alignment.CenterHorizontally)) {
                        Icon(icon, null, Modifier.size(20.dp))
                        Text(label, style = labelStyle, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        }
    }
}

@Composable
internal fun SftpPaneSurface(modifier: Modifier = Modifier, rounded: Boolean = false, content: @Composable () -> Unit) {
    Surface(
        modifier = modifier.fillMaxSize(),
        color = if (rounded) MaterialTheme.colorScheme.surfaceContainerLow else MaterialTheme.colorScheme.surfaceContainer,
        shape = if (rounded) MaterialTheme.shapes.extraLarge else RectangleShape,
        content = content
    )
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
    onRefresh: () -> Unit,
    refreshing: Boolean = false,
    searchVisible: Boolean,
    showSearchAction: Boolean,
    onOpenSearch: () -> Unit,
    onCloseSearch: () -> Unit,
    query: String,
    onQuery: (String) -> Unit,
    sort: FileSort,
    sortAscending: Boolean,
    onSort: (FileSort) -> Unit,
    onSelect: () -> Unit
) {
    val breadcrumbScroll = rememberScrollState()
    LaunchedEffect(path, breadcrumbScroll.maxValue) { breadcrumbScroll.animateScrollTo(breadcrumbScroll.maxValue) }
    Column(Modifier.fillMaxWidth()) {
        Surface(
            modifier = Modifier.fillMaxWidth().padding(horizontal = UiSpacing.xs),
            shape = CircleShape,
            color = MaterialTheme.colorScheme.surfaceContainerLowest,
            contentColor = MaterialTheme.colorScheme.onSurfaceVariant
        ) {
            if (searchVisible) SftpSearchField(query, onQuery, onCloseSearch) else Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(horizontal = UiSpacing.xxs),
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
        Row(
            Modifier.fillMaxWidth().padding(start = UiSpacing.sm, end = UiSpacing.xs),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text(if (refreshing && count == 0) stringResource(R.string.sftp_browser_loading_short)
                else stringResource(R.string.sftp_browser_item_count, count), Modifier.weight(1f),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (showSearchAction) IconButton(onClick = onOpenSearch) { Icon(Icons.Outlined.Search, stringResource(R.string.ux_filter_directory)) }
            SftpSortMenu(sort, sortAscending, onSort, onSelect, onRefresh, refreshing,
                onFilter = onOpenSearch, filterActive = query.isNotBlank(), onClearFilter = { onQuery("") })
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
private fun SftpSortMenu(sort: FileSort, ascending: Boolean, onSort: (FileSort) -> Unit, onSelect: () -> Unit,
    onRefresh: () -> Unit, refreshing: Boolean, onFilter: () -> Unit, filterActive: Boolean,
    onClearFilter: () -> Unit) {
    var menu by remember { mutableStateOf(false) }
    val optionsLabel = stringResource(R.string.sftp_browser_filter_sort)
    val direction = stringResource(if (ascending) R.string.sftp_browser_sort_ascending else R.string.sftp_browser_sort_descending)
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
    Box {
        TextButton(onClick = { menu = true },
            modifier = Modifier.padding(vertical = UiSpacing.xxs).testTag("sftp-directory-options")
                .semantics { contentDescription = optionsLabel; stateDescription = direction },
            colors = ButtonDefaults.textButtonColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLowest)) {
            Text(stringResource(sortCaption(sort)), style = MaterialTheme.typography.labelMedium)
            Spacer(Modifier.width(UiSpacing.xxs))
            Icon(if (ascending) Icons.Outlined.ArrowUpward else Icons.Outlined.ArrowDownward,
                null, Modifier.size(20.dp))
            Spacer(Modifier.width(UiSpacing.xxs))
            Icon(Icons.Outlined.Tune, null, Modifier.size(18.dp))
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false },
            modifier = Modifier.width(menuWidth).testTag("sftp-directory-options-menu"),
            shape = MaterialTheme.shapes.large,
            containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
            shadowElevation = 3.dp) {
            Text(stringResource(R.string.sftp_browser_sort_by),
                Modifier.padding(horizontal = UiSpacing.sm, vertical = UiSpacing.xs),
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min).padding(horizontal = UiSpacing.xs).selectableGroup(),
                horizontalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
                options.forEachIndexed { index, option ->
                    val widthWeight = preferredWidths[index].value
                    SftpSortOption(option, sort == option, ascending, { onSort(option) }, Modifier.weight(widthWeight).fillMaxHeight())
                }
            }
            HorizontalDivider(Modifier.padding(horizontal = UiSpacing.sm, vertical = UiSpacing.xs),
                color = MaterialTheme.colorScheme.outlineVariant)
            Text(stringResource(R.string.sftp_browser_options),
                Modifier.padding(horizontal = UiSpacing.sm, vertical = UiSpacing.xs),
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            DropdownMenuItem(text = { Text(stringResource(R.string.ux_filter_directory)) },
                onClick = { menu = false; onFilter() }, leadingIcon = { Icon(Icons.Outlined.Search, null) })
            if (filterActive) DropdownMenuItem(text = { Text(stringResource(R.string.sftp_browser_clear_search)) },
                onClick = { menu = false; onClearFilter() }, leadingIcon = { Icon(Icons.Outlined.FilterAltOff, null) })
            DropdownMenuItem(text = { Text(stringResource(R.string.ux_choose)) },
                onClick = { menu = false; onSelect() }, leadingIcon = { Icon(Icons.Outlined.CheckCircle, null) })
            DropdownMenuItem(text = { Text(stringResource(R.string.fm_refresh)) }, enabled = !refreshing,
                onClick = { menu = false; onRefresh() }, leadingIcon = { Icon(Icons.Outlined.Refresh, null) })
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

@Composable
internal fun SftpBrowserFooter(local: Boolean, showPaneSwitch: Boolean, onSelect: (Boolean) -> Unit,
    panePosition: () -> Float, transfers: List<SftpTransferModel>, onTransfers: () -> Unit) {
    val transferCountLabel = stringResource(R.string.sftp_browser_transfer_records, transfers.size)
    Column(
        Modifier.fillMaxWidth().navigationBarsPadding().padding(horizontal = UiSpacing.xs, vertical = UiSpacing.xs),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Column(Modifier.widthIn(max = 720.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
            if (transfers.any { !it.state.isTerminal || it.state == SftpTransferStateModel.Failed }) {
                SftpTransferDock(transfers, onTransfers)
            }
            Surface(shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.surfaceContainerLowest) {
                Row(Modifier.fillMaxWidth().padding(UiSpacing.xxs), verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
                    if (showPaneSwitch) SftpPaneSwitch(local, onSelect, panePosition, Modifier.weight(1f))
                    else Text(stringResource(R.string.sftp_transfers_title), Modifier.weight(1f).padding(start = UiSpacing.sm),
                        style = MaterialTheme.typography.labelLarge)
                    Box(Modifier.size(48.dp)) {
                        FilledTonalIconButton(onClick = onTransfers,
                            modifier = Modifier.size(48.dp).testTag("sftp-transfer-dock").semantics {
                                if (transfers.isNotEmpty()) stateDescription = transferCountLabel
                            },
                            shape = CircleShape, colors = IconButtonDefaults.filledTonalIconButtonColors(
                                containerColor = MaterialTheme.colorScheme.surfaceContainer,
                                contentColor = MaterialTheme.colorScheme.onSurfaceVariant)) {
                            Icon(Icons.Outlined.SwapVert, stringResource(R.string.sftp_transfers_title))
                        }
                        if (transfers.isNotEmpty()) Box(
                            Modifier.align(Alignment.TopEnd).offset(x = (-4).dp)
                                .background(MaterialTheme.colorScheme.surfaceContainerLowest, CircleShape)
                                .padding(2.dp).clearAndSetSemantics {}
                        ) {
                            Badge { Text(transfers.size.toString()) }
                        }
                    }
                }
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
internal fun SftpTransferDock(transfers: List<SftpTransferModel>, onClick: () -> Unit) {
    val active = transfers.filter { !it.state.isTerminal }
    val failed = transfers.count { it.state == SftpTransferStateModel.Failed }
    val progress = active.map { it.progress }.average().takeUnless { it.isNaN() }?.toFloat() ?: 0f
    Surface(
        onClick = onClick,
        modifier = Modifier.fillMaxWidth().testTag("sftp-transfer-summary"),
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerLowest
    ) {
        Column {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(horizontal = UiSpacing.sm, vertical = UiSpacing.xxs),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(UiSpacing.sm)
            ) {
                Icon(Icons.Outlined.SwapVert, null, tint = MaterialTheme.colorScheme.primary)
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        when {
                            active.isNotEmpty() || failed > 0 -> stringResource(R.string.ux_transfers, active.size, failed)
                            transfers.isNotEmpty() -> stringResource(R.string.sftp_browser_transfer_records, transfers.size)
                            else -> stringResource(R.string.sftp_browser_transfer_idle)
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = if (failed > 0) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1, overflow = TextOverflow.Ellipsis
                    )
                }
                if (active.isNotEmpty()) Text("${(progress * 100).toInt().coerceIn(0, 100)}%", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
                Icon(Icons.Outlined.ChevronRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (active.isNotEmpty()) LinearProgressIndicator(progress = { progress.coerceIn(0f, 1f) },
                modifier = Modifier.fillMaxWidth().padding(start = UiSpacing.sm, end = UiSpacing.sm, bottom = UiSpacing.xs))
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
