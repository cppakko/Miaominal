package dev.akko.miaominal.ui.components

import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.ArrowDownward
import androidx.compose.material.icons.outlined.ArrowUpward
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.DragHandle
import androidx.compose.material3.*
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.toSize
import androidx.compose.ui.zIndex
import dev.akko.miaominal.R
import dev.akko.miaominal.data.ProfileKind
import dev.akko.miaominal.data.SessionProfile

/** An ordered route draft; changes are persisted only by the profile's Save action. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun JumpChainEditor(
    hosts: List<SessionProfile>,
    targetId: String?,
    targetName: String,
    targetAddress: String,
    chain: List<String>,
    onChainChange: (List<String>) -> Unit,
    parentListState: LazyListState,
    viewport: Rect,
    enabled: Boolean
) {
    var pickerOpen by rememberSaveable { mutableStateOf(false) }
    val currentChain by rememberUpdatedState(chain)
    val changeChain by rememberUpdatedState(onChainChange)
    val rowBounds = remember { mutableStateMapOf<String, Rect>() }
    val handleBounds = remember { mutableMapOf<String, Rect>() }
    var draggedId by remember { mutableStateOf<String?>(null) }
    var pointerY by remember { mutableFloatStateOf(0f) }
    var grabOffset by remember { mutableFloatStateOf(0f) }
    val edge = with(LocalDensity.current) { 64.dp.toPx() }

    fun move(id: String, destination: Int) {
        val from = currentChain.indexOf(id)
        if (from < 0 || destination !in currentChain.indices || from == destination) return
        changeChain(currentChain.toMutableList().apply { add(destination, removeAt(from)) })
    }

    fun reorderAtPointer() {
        val id = draggedId ?: return
        val from = currentChain.indexOf(id)
        val next = currentChain.getOrNull(from + 1)?.let(rowBounds::get)
        val previous = currentChain.getOrNull(from - 1)?.let(rowBounds::get)
        when {
            next != null && pointerY > next.center.y -> move(id, from + 1)
            previous != null && pointerY < previous.center.y -> move(id, from - 1)
        }
    }

    val reorder by rememberUpdatedState(::reorderAtPointer)
    // Keep long chains usable when the pointer reaches the editor viewport edge.
    LaunchedEffect(draggedId, viewport) {
        if (draggedId == null) return@LaunchedEffect
        var previousFrame = withFrameNanos { it }
        while (draggedId != null) {
            val frame = withFrameNanos { it }
            val seconds = ((frame - previousFrame) / 1_000_000_000f).coerceAtMost(0.05f)
            previousFrame = frame
            val velocity = when {
                pointerY < viewport.top + edge -> -((viewport.top + edge - pointerY) / edge).coerceIn(0f, 1f)
                pointerY > viewport.bottom - edge -> ((pointerY - viewport.bottom + edge) / edge).coerceIn(0f, 1f)
                else -> 0f
            }
            if (velocity != 0f) parentListState.scrollBy(velocity * edge * 8 * seconds)
            reorder()
        }
    }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.forward_jump_chain), style = MaterialTheme.typography.titleMedium)
        Text(stringResource(R.string.jump_order_hint), style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (chain.isEmpty()) Text(stringResource(R.string.forward_jump_empty), style = MaterialTheme.typography.bodyMedium)
        chain.forEachIndexed { index, id ->
            key(id, chain.take(index).count { it == id }) {
                val host = hosts.firstOrNull { it.id == id }
                val label = host?.name ?: stringResource(R.string.jump_missing_host)
                val dragDescription = stringResource(R.string.jump_drag, label)
                val dragging = draggedId == id
                Box(Modifier.fillMaxWidth().zIndex(if (dragging) 1f else 0f)
                    .onGloballyPositioned { rowBounds[id] = Rect(it.positionInRoot(), it.size.toSize()) }) {
                    Surface(
                        modifier = Modifier.fillMaxWidth().testTag("jump-hop-$id").graphicsLayer {
                            translationY = if (dragging) pointerY - grabOffset - (rowBounds[id]?.top ?: 0f) else 0f
                            shadowElevation = if (dragging) 8.dp.toPx() else 0f
                        },
                        shape = MaterialTheme.shapes.medium,
                        color = if (dragging) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh
                    ) {
                        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(stringResource(R.string.jump_number, index + 1), style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.primary)
                            Text(label, style = MaterialTheme.typography.titleSmall)
                            Text(host?.let { "${it.username}@${it.host}:${it.port}" } ?: id,
                                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                                Box(Modifier.size(48.dp).testTag("jump-drag-$id")
                                    .onGloballyPositioned { handleBounds[id] = Rect(it.positionInRoot(), it.size.toSize()) }
                                    .semantics { contentDescription = dragDescription }
                                    .pointerInput(id, enabled) {
                                        if (enabled) detectDragGestures(
                                            onDragStart = { point ->
                                                pointerY = (handleBounds[id]?.top ?: 0f) + point.y
                                                grabOffset = pointerY - (rowBounds[id]?.top ?: 0f)
                                                draggedId = id
                                            },
                                            onDragEnd = { draggedId = null },
                                            onDragCancel = { draggedId = null },
                                            onDrag = { change, delta ->
                                                change.consume()
                                                pointerY += delta.y
                                                reorder()
                                            }
                                        )
                                    }, contentAlignment = Alignment.Center) {
                                    Icon(Icons.Outlined.DragHandle, contentDescription = null)
                                }
                                Spacer(Modifier.weight(1f))
                                IconButton(onClick = { move(id, index - 1) }, enabled = enabled && index > 0) {
                                    Icon(Icons.Outlined.ArrowUpward, stringResource(R.string.jump_move_up, label))
                                }
                                IconButton(onClick = { move(id, index + 1) }, enabled = enabled && index < chain.lastIndex) {
                                    Icon(Icons.Outlined.ArrowDownward, stringResource(R.string.jump_move_down, label))
                                }
                                IconButton(onClick = { onChainChange(chain.filterIndexed { i, _ -> i != index }) }, enabled = enabled) {
                                    Icon(Icons.Outlined.Close, stringResource(R.string.jump_remove, label))
                                }
                            }
                        }
                    }
                }
            }
        }
        OutlinedButton(onClick = { pickerOpen = true }, enabled = enabled, modifier = Modifier.fillMaxWidth()) {
            Icon(Icons.Outlined.Add, contentDescription = null)
            Spacer(Modifier.width(8.dp))
            Text(stringResource(R.string.forward_jump_add))
        }
        Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, shape = MaterialTheme.shapes.medium) {
            Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(stringResource(R.string.jump_destination), style = MaterialTheme.typography.labelMedium)
                Text(targetName.ifBlank { stringResource(R.string.jump_unnamed_target) }, style = MaterialTheme.typography.titleSmall)
                if (targetAddress.isNotBlank()) Text(targetAddress, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
    if (pickerOpen) {
        JumpHostPicker(
            candidates = hosts.filter { it.kind == ProfileKind.Ssh && it.id != targetId && it.id !in chain },
            onDismiss = { pickerOpen = false },
            onAdd = { ids -> onChainChange((chain + ids).distinct()); pickerOpen = false }
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun JumpHostPicker(candidates: List<SessionProfile>, onDismiss: () -> Unit, onAdd: (List<String>) -> Unit) {
    var query by rememberSaveable { mutableStateOf("") }
    var selected by rememberSaveable { mutableStateOf(listOf<String>()) }
    val availableIds = candidates.map { it.id }.toSet()
    val selection = selected.filter { it in availableIds }
    val visible = candidates.filter { host ->
        listOf(host.name, host.host, host.username, host.group).plus(host.tags).any { it.contains(query.trim(), true) }
    }.sortedBy { it.name.lowercase() }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().heightIn(max = 620.dp).imePadding().padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.jump_picker_title), style = MaterialTheme.typography.titleLarge)
            Text(stringResource(R.string.jump_picker_hint), style = MaterialTheme.typography.bodySmall)
            OutlinedTextField(query, onValueChange = { query = it }, singleLine = true,
                label = { Text(stringResource(R.string.jump_search)) }, modifier = Modifier.fillMaxWidth())
            LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (visible.isEmpty()) item {
                    Text(stringResource(if (candidates.isEmpty()) R.string.jump_no_candidates else R.string.search_no_hosts),
                        modifier = Modifier.padding(vertical = 16.dp))
                }
                items(visible, key = { it.id }) { host ->
                    ListItem(
                        modifier = Modifier.testTag("jump-candidate-${host.id}").toggleable(host.id in selection, role = Role.Checkbox, onValueChange = { checked ->
                            selected = if (checked) selection + host.id else selection - host.id
                        }),
                        headlineContent = { Text(host.name) },
                        supportingContent = {
                            Column {
                                Text("${host.username}@${host.host}:${host.port}", maxLines = 2, overflow = TextOverflow.Ellipsis)
                                if (host.group.isNotBlank()) Text(host.group)
                            }
                        },
                        trailingContent = { Checkbox(checked = host.id in selection, onCheckedChange = null) }
                    )
                }
            }
            Button(onClick = { onAdd(selection) }, enabled = selection.isNotEmpty(), modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.jump_add_selected, selection.size))
            }
            Spacer(Modifier.height(8.dp))
        }
    }
}
