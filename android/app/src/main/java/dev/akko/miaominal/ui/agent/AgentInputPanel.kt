package dev.akko.miaominal.ui.agent

import dev.akko.miaominal.ui.theme.UiSpacing
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.data.agent.*

/** A single bounded composer: keyboard space is reserved for the draft, not settings rows. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AgentInputPanel(
    state: AgentSessionState,
    draft: String,
    files: List<String>,
    onDraftChange: (String) -> Unit,
    onSend: () -> Unit,
    onStop: () -> Unit,
    onAttachFile: (() -> Unit)?,
    onRemoveFile: (String) -> Unit,
    onSelectModel: (AgentModel) -> Unit,
    onSelectReasoning: (String) -> Unit,
    onInteraction: () -> Unit,
    compact: Boolean = false
) {
    var preferences by rememberSaveable { mutableStateOf(false) }
    var selectedPath by rememberSaveable { mutableStateOf<String?>(null) }
    var fileHelp by rememberSaveable { mutableStateOf(false) }
    var more by remember { mutableStateOf(false) }
    val online = state.phase == AgentPhase.Ready && !state.connectionLost && state.session?.remoteSessionId != null
    val keyboard = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    val inputLabel = stringResource(R.string.ui_enter_a_message)
    val reasoning = state.reasoningOptions.firstOrNull { it.id == state.selectedReasoning }?.label
    Surface(
        modifier = Modifier.widthIn(max = 840.dp).fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = if (compact) 4.dp else 8.dp),
        color = MaterialTheme.colorScheme.surfaceContainer,
        shape = RoundedCornerShape(28.dp)
    ) {
        Column(Modifier.padding(horizontal = 6.dp, vertical = UiSpacing.xxs)) {
            state.pendingInteraction?.let {
                TextButton(onClick = onInteraction) {
                    Text(stringResource(R.string.agent_pending_question, it.title), maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            if (!compact) Text(listOfNotNull(state.selectedModel?.name, reasoning).joinToString(" · "),
                modifier = Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (!compact && files.isNotEmpty()) LazyRow(horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                items(files, key = { it }) { path ->
                    InputChip(
                        selected = false,
                        onClick = { selectedPath = path },
                        label = { Text(path.substringAfterLast('/').substringAfterLast('\\'),
                            maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 160.dp)) },
                        trailingIcon = {
                            IconButton(onClick = { onRemoveFile(path) }) {
                                Icon(Icons.Outlined.Close, stringResource(R.string.agent_remove_file, path))
                            }
                        }
                    )
                }
            }
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                if (!compact && onAttachFile != null) IconButton(onClick = {
                    if (files.size >= 8 || !online) fileHelp = true else onAttachFile()
                }, enabled = !state.isBusy, modifier = Modifier.size(48.dp)) {
                    Icon(Icons.Outlined.Add, stringResource(R.string.agent_reference_count, files.size),
                        modifier = Modifier.size(24.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                BasicTextField(
                    value = draft,
                    onValueChange = onDraftChange,
                    textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                    maxLines = if (compact) 1 else if (keyboard) 3 else 5,
                    modifier = Modifier.weight(1f).heightIn(min = 48.dp)
                        .padding(horizontal = 6.dp, vertical = 12.dp).testTag(AgentComposerTag).semantics { contentDescription = inputLabel },
                    decorationBox = { innerTextField ->
                        Box(contentAlignment = Alignment.CenterStart) {
                            if (draft.isEmpty()) Text(stringResource(R.string.ui_enter_a_message),
                                style = MaterialTheme.typography.bodyLarge,
                                color = MaterialTheme.colorScheme.onSurfaceVariant)
                            innerTextField()
                        }
                    }
                )
                if (!compact) IconButton(onClick = { preferences = true }, modifier = Modifier.size(48.dp)) {
                    Icon(Icons.Outlined.Tune,
                        listOfNotNull(stringResource(R.string.agent_preferences), state.selectedModel?.name, reasoning)
                            .joinToString(" · "),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(22.dp))
                }
                if (compact) Box {
                    IconButton(onClick = { more = true }, modifier = Modifier.size(48.dp)) {
                        Icon(Icons.Outlined.MoreVert, stringResource(R.string.ux_models))
                    }
                    DropdownMenu(expanded = more, onDismissRequest = { more = false }) {
                        DropdownMenuItem(text = { Text(stringResource(R.string.agent_preferences)) }, onClick = { more = false; preferences = true })
                        if (onAttachFile != null) DropdownMenuItem(text = { Text(stringResource(R.string.agent_reference_count, files.size)) },
                            enabled = !state.isBusy, onClick = { more = false; if (files.size >= 8 || !online) fileHelp = true else onAttachFile() })
                        files.forEach { path -> DropdownMenuItem(text = { Text(path.substringAfterLast('/')) },
                            onClick = { more = false; selectedPath = path }) }
                    }
                }
                AgentSendControl(state.isBusy, online && (draft.isNotBlank() || files.isNotEmpty()), onSend, onStop)
            }
        }
    }
    selectedPath?.let { path ->
        AlertDialog(onDismissRequest = { selectedPath = null },
            title = { Text(stringResource(R.string.agent_full_path)) },
            text = { SelectionContainer { Text(path) } },
            confirmButton = { TextButton(onClick = { selectedPath = null }) { Text(stringResource(R.string.agent_close_detail)) } },
            dismissButton = { Row { AgentCopyButton(path); TextButton(onClick = { onRemoveFile(path); selectedPath = null }) {
                Text(stringResource(R.string.agent_remove_file, path))
            } } })
    }
    if (fileHelp) AlertDialog(onDismissRequest = { fileHelp = false },
        title = { Text(stringResource(R.string.agent_reference_count, files.size)) },
        text = { Text(stringResource(R.string.agent_reference_help)) },
        confirmButton = { TextButton(onClick = { fileHelp = false }) { Text(stringResource(R.string.agent_close_detail)) } })
    if (preferences) {
        var query by rememberSaveable { mutableStateOf("") }
        ModalBottomSheet(onDismissRequest = { preferences = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 560.dp).imePadding(),
                contentPadding = PaddingValues(20.dp), verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                item { Text(stringResource(R.string.agent_preferences), style = MaterialTheme.typography.titleLarge) }
                item { Text(listOfNotNull(state.selectedModel?.name, reasoning).joinToString(" · "),
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                if (state.reasoningOptions.isNotEmpty()) {
                    item { Text(stringResource(R.string.agent_reasoning_help), style = MaterialTheme.typography.bodySmall) }
                    item {
                        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                            state.reasoningOptions.forEach { option ->
                                FilterChip(selected = state.selectedReasoning == option.id,
                                    enabled = online && !state.isBusy,
                                    onClick = { onSelectReasoning(option.id) }, label = { Text(option.label) })
                            }
                        }
                    }
                }
                if (state.capabilities.modelSelection && state.models.isNotEmpty()) {
                    item { OutlinedTextField(query, { query = it }, singleLine = true,
                        label = { Text(stringResource(R.string.agent_search_models)) }, modifier = Modifier.fillMaxWidth()) }
                    items(state.models.filter { it.name.contains(query, true) }, key = { it.key }) { model ->
                        TextButton(onClick = { onSelectModel(model) }, enabled = online && !state.isBusy,
                            modifier = Modifier.fillMaxWidth()) {
                            RadioButton(selected = state.selectedModel?.key == model.key, onClick = null)
                            Text(model.name, Modifier.weight(1f).padding(start = 8.dp))
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun AgentSendControl(busy: Boolean, canSend: Boolean, onSend: () -> Unit, onStop: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    IconButton(onClick = if (busy) onStop else onSend, enabled = busy || canSend,
        modifier = Modifier.size(48.dp).then(if (busy) Modifier else Modifier.testTag(AgentSendTag))) {
        Surface(shape = CircleShape, modifier = Modifier.size(36.dp),
            color = if (busy || canSend) colors.onSurface else colors.onSurface.copy(alpha = 0.08f)) {
            Box(contentAlignment = Alignment.Center) {
                Icon(if (busy) Icons.Outlined.Stop else Icons.Outlined.ArrowUpward,
                    stringResource(if (busy) R.string.agent_stop else R.string.ui_send),
                    tint = if (busy || canSend) colors.surface else colors.onSurface.copy(alpha = 0.38f),
                    modifier = Modifier.size(20.dp))
            }
        }
    }
}
