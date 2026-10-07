package dev.akko.miaominal.ui.components

import android.widget.Toast
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Article
import androidx.compose.material.icons.automirrored.outlined.HelpOutline
import androidx.compose.material.icons.automirrored.outlined.Undo
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.*
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.*
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.theme.UiSpacing
import dev.akko.miaominal.ui.theme.MiaominalMotion
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import dev.akko.miaominal.ui.components.HintIcon as Icon

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TerminalTextExplosionDialog(document: TerminalExplosionSelection?, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val topBarHeight = with(LocalDensity.current) {
        (MaterialTheme.typography.titleLarge.lineHeight.toDp() +
            MaterialTheme.typography.labelMedium.lineHeight.toDp() + UiSpacing.sm).coerceAtLeast(64.dp)
    }
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(Unit) { keyboard?.hide() }
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    var original by rememberSaveable { mutableStateOf(false) }
    var more by remember { mutableStateOf(false) }
    val result = remember(document, document?.version) { document?.resultText().orEmpty() }
    val selectedCount = remember(document, document?.version) { document?.selectedTokenCount ?: 0 }
    val haptic = LocalHapticFeedback.current
    val tokenScroll = rememberLazyListState()
    fun feedback(message: String) { scope.launch { snackbar.showSnackbar(message) } }
    fun copy(close: Boolean) {
        if (result.isBlank()) return
        copyTerminalText(context, result).fold(
            onSuccess = {
                haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                if (close) {
                    Toast.makeText(context, R.string.terminal_explosion_copied, Toast.LENGTH_SHORT).show()
                    onDismiss()
                } else feedback(context.getString(R.string.terminal_explosion_copied))
            },
            onFailure = { feedback(context.getString(R.string.explosion_copy_failed)) }
        )
    }

    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(
        usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        BoxWithConstraints(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) {
            val floating = maxWidth >= 600.dp
            val margin = if (floating) UiSpacing.md else 0.dp
            Surface(
                modifier = Modifier.padding(margin).widthIn(max = 1040.dp).heightIn(max = 900.dp).fillMaxSize()
                    .testTag("explosion-editor"),
                shape = if (floating) MaterialTheme.shapes.extraLarge else androidx.compose.ui.graphics.RectangleShape,
                color = MaterialTheme.colorScheme.surface
            ) {
                Scaffold(
                    contentWindowInsets = WindowInsets(0),
                    snackbarHost = { SnackbarHost(snackbar) },
                    topBar = {
                        TopAppBar(expandedHeight = topBarHeight, title = {
                            Column {
                                Text(stringResource(R.string.terminal_explosion_title),
                                    style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold,
                                    maxLines = 1, overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.semantics { heading() })
                                Text(if (document == null) stringResource(R.string.explosion_loading) else
                                    stringResource(R.string.terminal_explosion_selected_count, selectedCount, document.tokens.size),
                                    style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    maxLines = 1, overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.testTag("explosion-count"))
                            }
                        }, navigationIcon = {
                            IconButton(onClick = onDismiss) {
                                Icon(Icons.Outlined.Close, stringResource(R.string.terminal_explosion_close))
                            }
                        }, actions = {
                            IconButton(onClick = { original = true }, enabled = document != null,
                                modifier = Modifier.testTag("explosion-original")) {
                                Icon(Icons.AutoMirrored.Outlined.Article, stringResource(R.string.explosion_original))
                            }
                        })
                    },
                    bottomBar = {
                        Surface(color = MaterialTheme.colorScheme.surfaceContainer) {
                            Column(Modifier.padding(horizontal = UiSpacing.sm, vertical = UiSpacing.xs),
                                verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                                    Button(onClick = { copy(true) }, enabled = result.isNotBlank(),
                                        colors = ButtonDefaults.buttonColors(
                                            disabledContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                                            disabledContentColor = MaterialTheme.colorScheme.onSurfaceVariant),
                                        modifier = Modifier.weight(1f).heightIn(min = 48.dp).testTag("explosion-copy-return")) {
                                        Icon(Icons.Outlined.ContentCopy, null, Modifier.size(20.dp))
                                        Spacer(Modifier.width(UiSpacing.xs))
                                        Text(stringResource(R.string.explosion_copy_return))
                                    }
                                    Box {
                                        FilledTonalIconButton(onClick = { more = true }, modifier = Modifier.size(48.dp).testTag("explosion-more")) {
                                            Icon(Icons.Outlined.MoreHoriz, stringResource(R.string.explosion_more))
                                        }
                                        DropdownMenu(expanded = more, onDismissRequest = { more = false }) {
                                            DropdownMenuItem(text = { Text(stringResource(R.string.explosion_copy_keep)) },
                                                enabled = result.isNotBlank(), onClick = { more = false; copy(false) },
                                                leadingIcon = { Icon(Icons.Outlined.ContentCopy, null) })
                                            DropdownMenuItem(text = { Text(stringResource(R.string.explosion_web_search)) },
                                                enabled = result.isNotBlank(), onClick = {
                                                    more = false
                                                    searchTerminalText(context, result).onFailure { feedback(context.getString(R.string.explosion_search_failed)) }
                                                }, leadingIcon = { Icon(Icons.Outlined.Search, null) })
                                            DropdownMenuItem(text = { Text(stringResource(R.string.terminal_explosion_share)) },
                                                enabled = result.isNotBlank(), onClick = {
                                                    more = false
                                                    shareTerminalText(context, result).onFailure { feedback(context.getString(R.string.explosion_share_failed)) }
                                                }, leadingIcon = { Icon(Icons.Outlined.Share, null) })
                                        }
                                    }
                                }
                            }
                        }
                    }
                ) { padding ->
                    if (document == null) Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                        CircularProgressIndicator(modifier = Modifier.semantics { contentDescription = context.getString(R.string.explosion_loading) })
                    } else ExplosionSelectionPane(document, tokenScroll, Modifier.fillMaxSize().padding(padding))
                }
            }
        }
        if (original && document != null) AlertDialog(onDismissRequest = { original = false },
            title = { Text(stringResource(R.string.explosion_original)) },
            text = { SelectionContainer { Text(document.source,
                modifier = Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()).testTag("explosion-original-text"),
                fontFamily = FontFamily.Monospace) } },
            confirmButton = { TextButton(onClick = { original = false }) { Text(stringResource(R.string.profile_close)) } })
    }
}

@Composable
internal fun ExplosionSelectionPane(document: TerminalExplosionSelection, listState: LazyListState, modifier: Modifier) {
    BoxWithConstraints(modifier) {
        // Use the pane's actual available height, after the app bar and copy action.
        val pinned = maxHeight >= 480.dp
        Column(Modifier.fillMaxSize()) {
            if (pinned) Surface(color = MaterialTheme.colorScheme.surface) {
                ExplosionSelectionControls(document, Modifier.padding(horizontal = UiSpacing.sm)
                    .testTag("explosion-pinned-controls"))
            }
            ExplosionTokenList(document, listState, Modifier.weight(1f).fillMaxWidth(), scrollControls = !pinned)
        }
    }
}

@Composable
private fun ExplosionSelectionControls(document: TerminalExplosionSelection, modifier: Modifier = Modifier) {
    val haptic = LocalHapticFeedback.current
    Column(modifier.padding(vertical = UiSpacing.xxs), verticalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
        FlowRow(horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
            TextButton(onClick = { document.selectAll(); haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove) },
                enabled = !document.allSelected && document.tokens.isNotEmpty(), modifier = Modifier.heightIn(min = 48.dp).testTag("explosion-all")) {
                Text(stringResource(R.string.explosion_all))
            }
            TextButton(onClick = { document.clear(); haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove) },
                enabled = !document.isEmpty, modifier = Modifier.heightIn(min = 48.dp).testTag("explosion-clear")) {
                Text(stringResource(R.string.explosion_clear))
            }
            TextButton(onClick = { document.undo() }, enabled = document.canUndo,
                modifier = Modifier.heightIn(min = 48.dp).testTag("explosion-undo")) {
                Icon(Icons.AutoMirrored.Outlined.Undo, null, Modifier.size(18.dp)); Spacer(Modifier.width(UiSpacing.xxs))
                Text(stringResource(R.string.explosion_undo))
            }
        }
    }
}

@Composable
private fun ExplosionTokenList(document: TerminalExplosionSelection, listState: LazyListState, modifier: Modifier, scrollControls: Boolean) {
    val tokens = document.tokens
    val bounds = remember(tokens) { mutableMapOf<Int, Rect>() }
    val drag = remember(document, tokens) { ExplosionDragState() }
    val scope = rememberCoroutineScope()
    val haptic = LocalHapticFeedback.current
    val edge = with(LocalDensity.current) { 48.dp.toPx() }
    fun visit(position: Offset) {
        val point = position.copy(y = position.y.coerceIn(0f, (drag.height - 1f).coerceAtLeast(0f)))
        val index = bounds.entries.firstOrNull { it.value.contains(point + drag.origin) }?.key ?: return
        drag.selection?.visit(index)
    }
    fun finishDrag() { drag.position = null; drag.selection = null; drag.job?.cancel(); drag.job = null }
    DisposableEffect(document, tokens) { onDispose { finishDrag() } }
    LazyColumn(state = listState, contentPadding = PaddingValues(UiSpacing.sm),
        verticalArrangement = Arrangement.spacedBy(UiSpacing.xs), modifier = modifier.testTag("explosion-tokens")
            .onGloballyPositioned { drag.origin = it.positionInRoot(); drag.height = it.size.height.toFloat() }
            .pointerInput(document, tokens, edge) {
                detectDragGesturesAfterLongPress(onDragStart = { position ->
                    val index = bounds.entries.firstOrNull { it.value.contains(position + drag.origin) }?.key
                    if (index != null) {
                        drag.selection = TerminalExplosionDragSelection(document)
                        visit(position)
                        drag.position = position
                        haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                        drag.job = scope.launch {
                            var previousFrame = 0L
                            while (drag.position != null) {
                                val now = withFrameNanos { it }
                                val seconds = if (previousFrame == 0L) 1f / 60f else
                                    ((now - previousFrame) / 1_000_000_000f).coerceIn(0f, 0.05f)
                                previousFrame = now
                                val point = drag.position ?: break
                                val direction = when {
                                    point.y < edge -> -((edge - point.y) / edge).coerceIn(0f, 1f)
                                    point.y > drag.height - edge -> ((point.y - drag.height + edge) / edge).coerceIn(0f, 1f)
                                    else -> 0f
                                }
                                if (direction != 0f) listState.scrollBy(direction * edge * 15f * seconds)
                                visit(point)
                            }
                        }
                    }
                }, onDrag = { change, _ ->
                    if (drag.position != null) { change.consume(); drag.position = change.position; visit(change.position) }
                }, onDragEnd = ::finishDrag, onDragCancel = ::finishDrag)
            }) {
        if (scrollControls) item(key = "controls") {
            ExplosionSelectionControls(document, Modifier.testTag("explosion-scrolling-controls"))
        }
        item(key = "hint") { ExplosionGestureHint() }
        if (tokens.isEmpty()) item { Text(stringResource(R.string.explosion_no_tokens)) }
        // Bound composition/measurement work even for thousands of words.
        (tokens.indices step 48).forEach { first ->
            item(key = first) {
                BoxWithConstraints(Modifier.fillMaxWidth()) {
                    val tokenMaxWidth = maxWidth
                    FlowRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs),
                        verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                        for (index in first until minOf(first + 48, tokens.size)) {
                            val state = document.tokenState(index)
                            DisposableEffect(tokens, index) { onDispose { bounds.remove(index) } }
                            ExplosionToken(document, index, state,
                                onClick = {
                                    document.setTokenSelected(index, state != ToggleableState.On)
                                    haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                                }, modifier = Modifier.heightIn(min = 48.dp)
                                    .widthIn(min = 48.dp, max = tokenMaxWidth.coerceAtLeast(48.dp))
                                    .testTag("explosion-token-$index")
                                    .onGloballyPositioned { bounds[index] = it.boundsInRoot() })
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ExplosionGestureHint() {
    var help by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(stringResource(R.string.explosion_gesture_hint), modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        IconButton(onClick = { help = true }, modifier = Modifier.testTag("explosion-help")) {
            Icon(Icons.AutoMirrored.Outlined.HelpOutline, stringResource(R.string.explosion_selection_help))
        }
    }
    if (help) AlertDialog(onDismissRequest = { help = false },
        title = { Text(stringResource(R.string.explosion_selection_help)) },
        text = { Text(stringResource(R.string.explosion_gesture_help)) },
        confirmButton = { TextButton(onClick = { help = false }) { Text(stringResource(R.string.profile_close)) } })
}

@Composable
private fun ExplosionToken(document: TerminalExplosionSelection, index: Int, state: ToggleableState,
    onClick: () -> Unit, modifier: Modifier) {
    val colors = MaterialTheme.colorScheme
    val full = state == ToggleableState.On
    val container by animateColorAsState(
        if (full) colors.primaryContainer else colors.surfaceContainerLow,
        tween(MiaominalMotion.DurationShort2), label = "explosion-token-container")
    val foreground by animateColorAsState(
        if (full) colors.onPrimaryContainer else colors.onSurface,
        tween(MiaominalMotion.DurationShort2), label = "explosion-token-foreground")
    val border by animateColorAsState(
        if (full) colors.primary else colors.outlineVariant,
        tween(MiaominalMotion.DurationShort2), label = "explosion-token-border")
    Surface(onClick = onClick, modifier = modifier.semantics {
        role = Role.Checkbox
        toggleableState = state
    }, shape = MaterialTheme.shapes.medium, color = container, contentColor = foreground,
        border = BorderStroke(1.dp, border)) {
        Box(Modifier.padding(horizontal = UiSpacing.xs + UiSpacing.xxs, vertical = UiSpacing.xs),
            contentAlignment = Alignment.CenterStart) {
            Text(document.tokens[index].text, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Normal, softWrap = true)
        }
    }
}

private class ExplosionDragState {
    var origin = Offset.Zero
    var height = 0f
    var position: Offset? = null
    var job: Job? = null
    var selection: TerminalExplosionDragSelection? = null
}
