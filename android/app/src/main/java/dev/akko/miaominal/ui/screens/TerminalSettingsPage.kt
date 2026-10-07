package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DragHandle
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.data.DefaultTerminalQuickKeys
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.HintIcon as Icon
import dev.akko.miaominal.ui.components.SectionTitle
import dev.akko.miaominal.ui.model.RemovedTerminalKey
import dev.akko.miaominal.ui.model.restoreTerminalKey
import dev.akko.miaominal.ui.model.reorderTerminalKey
import kotlinx.coroutines.launch

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun MobileTerminalSettingsPage(modifier: Modifier, focusAnchor: String?) {
    val repository = LocalMiaominalRepository.current
    val preferences by repository.devicePreferences.collectAsState()
    val scope = rememberCoroutineScope()
    val list = rememberLazyListState()
    val snackbar = remember { SnackbarHostState() }
    var reset by rememberSaveable { mutableStateOf(false) }
    var error by rememberSaveable { mutableStateOf<String?>(null) }
    var values by remember { mutableStateOf(normalizeTerminalQuickKeys(preferences.terminalQuickKeys)) }
    var dragging by remember { mutableStateOf<String?>(null) }
    var dragPosition by remember { mutableFloatStateOf(0f) }
    var menu by rememberSaveable { mutableStateOf<String?>(null) }
    val edge = with(LocalDensity.current) { 48.dp.toPx() }
    val undoLabel = stringResource(R.string.settings_undo)
    val removedLabel = stringResource(R.string.settings_key_removed)
    LaunchedEffect(preferences.terminalQuickKeys) { if (dragging == null) values = normalizeTerminalQuickKeys(preferences.terminalQuickKeys) }
    SettingsAnchorScroll(list, focusAnchor)
    fun save(next: List<String>) {
        values = normalizeTerminalQuickKeys(next)
        val snapshot = values
        scope.launch { try { repository.updateDevicePreferences { it.copy(terminalQuickKeys = snapshot) } }
            catch (failure: Exception) { error = describeSyncError(failure); values = normalizeTerminalQuickKeys(repository.devicePreferences.value.terminalQuickKeys) } }
    }
    fun move(key: String, delta: Int) {
        val index = values.indexOf(key); val target = index + delta
        if (index >= 0 && target in values.indices) save(reorderTerminalKey(values, key, target))
    }
    fun remove(key: String) {
        if (values.size <= 1 || key !in values) return
        val removed = RemovedTerminalKey(key, values.indexOf(key)); save(values - key)
        scope.launch { if (snackbar.showSnackbar(removedLabel, undoLabel, withDismissAction = true) == SnackbarResult.ActionPerformed)
            save(restoreTerminalKey(normalizeTerminalQuickKeys(repository.devicePreferences.value.terminalQuickKeys), removed)) }
    }
    Box(modifier) {
        LazyColumn(state = list, modifier = Modifier.fillMaxSize(), contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            item(key = "terminal-typography") { SectionTitle(stringResource(R.string.desktop_appearance_typography_group), focused = focusAnchor == "terminal-typography") }
            item { TerminalTypographySettings() }
            item(key = "terminal-preview") { SectionTitle(stringResource(R.string.terminal_shortcuts_preview)) }
            item { Text(values.mapNotNull { value -> AllTerminalKeys.firstOrNull { it.value == value }?.label }.joinToString("  ·  "), style = MaterialTheme.typography.bodyLarge) }
            item(key = "terminal-quick") { SectionTitle(stringResource(R.string.terminal_shortcuts_quick_group)) }
            item { Text(stringResource(R.string.settings_drag_keys), style = MaterialTheme.typography.bodyMedium) }
            itemsIndexed(values, key = { _, value -> "shortcut:$value" }) { index, value ->
                val key = AllTerminalKeys.first { it.value == value }
                val up = stringResource(R.string.terminal_shortcut_move_up, key.label)
                val down = stringResource(R.string.terminal_shortcut_move_down, key.label)
                val delete = stringResource(R.string.terminal_shortcut_remove, key.label)
                ListItem(headlineContent = { Text(key.label) }, supportingContent = { Text(stringResource(key.description)) },
                    leadingContent = { Icon(Icons.Outlined.DragHandle, stringResource(R.string.settings_drag_keys)) },
                    trailingContent = { Box {
                        IconButton(onClick = { menu = value }) { Icon(Icons.Outlined.MoreVert, stringResource(R.string.settings_key_actions, key.label)) }
                        DropdownMenu(expanded = menu == value, onDismissRequest = { menu = null }) {
                            DropdownMenuItem(text = { Text(up) }, enabled = index > 0, onClick = { menu = null; move(value, -1) })
                            DropdownMenuItem(text = { Text(down) }, enabled = index < values.lastIndex, onClick = { menu = null; move(value, 1) })
                            DropdownMenuItem(text = { Text(delete) }, enabled = values.size > 1, onClick = { menu = null; remove(value) })
                        }
                    } }, colors = ListItemDefaults.colors(containerColor = if (dragging == value) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surfaceContainerLow),
                    modifier = Modifier.animateItem().clip(MaterialTheme.shapes.large).semantics {
                        customActions = buildList {
                            if (index > 0) add(CustomAccessibilityAction(up) { move(value, -1); true })
                            if (index < values.lastIndex) add(CustomAccessibilityAction(down) { move(value, 1); true })
                            if (values.size > 1) add(CustomAccessibilityAction(delete) { remove(value); true })
                        }
                    }.pointerInput(value) {
                        detectDragGesturesAfterLongPress(onDragStart = { offset ->
                            dragging = value
                            dragPosition = (list.layoutInfo.visibleItemsInfo.firstOrNull { it.key == "shortcut:$value" }?.offset ?: 0) + offset.y
                        }, onDragCancel = { dragging = null; values = normalizeTerminalQuickKeys(repository.devicePreferences.value.terminalQuickKeys) },
                            onDragEnd = { dragging = null; save(values) }, onDrag = { change, delta ->
                                change.consume(); dragPosition += delta.y
                                val info = list.layoutInfo
                                val targetKey = info.visibleItemsInfo.firstOrNull { dragPosition >= it.offset && dragPosition < it.offset + it.size }?.key as? String
                                val target = targetKey?.removePrefix("shortcut:")?.let(values::indexOf) ?: -1
                                if (target >= 0) values = reorderTerminalKey(values, value, target)
                                val scrollDelta = when { dragPosition > info.viewportEndOffset - edge -> edge / 3; dragPosition < info.viewportStartOffset + edge -> -edge / 3; else -> 0f }
                                if (scrollDelta != 0f) list.dispatchRawDelta(scrollDelta)
                            })
                    })
            }
            item(key = "terminal-more") { SectionTitle(stringResource(R.string.terminal_shortcuts_more_group)) }
            item { FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                AllTerminalKeys.filterNot { it.value in values }.forEach { key -> FilterChip(false, onClick = { save(values + key.value) }, label = { Text(key.label) }) }
            } }
            item { OutlinedButton(onClick = { reset = true }) { Text(stringResource(R.string.settings_reset_keys)) } }
            item { error?.let { SettingsErrorText(it) }; Spacer(Modifier.height(64.dp)) }
        }
        SnackbarHost(snackbar, modifier = Modifier.align(Alignment.BottomCenter))
    }
    if (reset) AlertDialog(onDismissRequest = { reset = false }, title = { Text(stringResource(R.string.settings_reset_keys)) },
        text = { Text(stringResource(R.string.settings_reset_keys_hint)) },
        confirmButton = { TextButton(onClick = { reset = false; save(DefaultTerminalQuickKeys) }) { Text(stringResource(R.string.settings_reset_keys)) } },
        dismissButton = { TextButton(onClick = { reset = false }) { Text(stringResource(R.string.tool_action_cancel)) } })
}
