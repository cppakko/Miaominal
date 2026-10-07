package dev.akko.miaominal.ui.screens

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.MiaominalApplication
import dev.akko.miaominal.R
import dev.akko.miaominal.data.*
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.model.SettingsOperations

@Composable
internal fun SettingsImportFlow(operations: SettingsOperations) {
    val repository = LocalMiaominalRepository.current
    val context = LocalContext.current
    val policy = (context.applicationContext as? MiaominalApplication)?.container?.backgroundPolicy
    var query by rememberSaveable { mutableStateOf("") }
    var confirm by rememberSaveable { mutableStateOf(false) }
    val previewList = rememberLazyListState()
    LaunchedEffect(operations.importError) { if (operations.importError != null) previewList.animateScrollToItem(1) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        policy?.endUserFlow()
        if (uri != null) operations.readImport(repository, context.applicationContext.contentResolver, uri)
    }
    val choose = { policy?.beginUserFlow(); launcher.launch(arrayOf("*/*")) }
    if (!operations.importOpen) return
    val preview = operations.preview
    val busy = operations.importReading || operations.importCommitting
    SettingsEditorPanel(stringResource(R.string.desktop_connections_import_group), operations.importCommitting,
        onDismiss = { operations.closeImport(repository) }, scroll = false) {
        val count = operations.importedCount
        LazyColumn(state = previewList, modifier = Modifier.weight(1f).testTag("settings-import-preview"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            item { Text(stringResource(R.string.settings_import_steps), style = MaterialTheme.typography.labelLarge) }
            item { operations.importError?.let { SettingsErrorText(when (it) {
                is ImportDocuments.TooLargeException -> stringResource(R.string.ui_the_selected_file_exceeds_the_8_mib_import_limit)
                is SecurityException -> stringResource(R.string.settings_import_authorization)
                is java.io.IOException -> stringResource(R.string.settings_import_read_error)
                else -> describeSyncError(it)
            }) } }
            if (count != null) {
                item { Text(stringResource(R.string.settings_import_result, count), style = MaterialTheme.typography.headlineSmall) }
            } else if (preview == null) {
                itemsIndexed(ImportSourceModel.entries) { _, source ->
                    FilterChip(selected = operations.source == source, enabled = !busy,
                        onClick = { operations.chooseImportSource(repository, source) }, label = { Text(stringResource(when (source) {
                            ImportSourceModel.OpenSsh -> R.string.desktop_connections_import_openssh
                            ImportSourceModel.Putty -> R.string.desktop_connections_import_putty
                            ImportSourceModel.SecureCrt -> R.string.desktop_connections_import_securecrt
                            ImportSourceModel.FinalShell -> R.string.desktop_connections_import_finalshell
                        })) })
                }
                }
            if (count == null) {
                item { Button(enabled = !busy, onClick = choose) { Text(stringResource(R.string.settings_select_file)) } }
                item { if (operations.importReading) LinearProgressIndicator(Modifier.fillMaxWidth()) }
            }
            if (preview != null && count == null) {
                item { Text(stringResource(R.string.settings_import_batch_hint, preview.entries.size, preview.warnings.size)) }
                item { OutlinedTextField(value = query, onValueChange = { query = it }, enabled = !busy,
                    label = { Text(stringResource(R.string.settings_preview_search)) }, modifier = Modifier.fillMaxWidth(), singleLine = true) }
                    itemsIndexed(preview.entries.filter { query.isBlank() || "${it.name} ${it.host} ${it.username} ${it.group}".contains(query, true) }) { index, entry ->
                        ListItem(headlineContent = { Text(entry.name) }, supportingContent = {
                            Text(buildString {
                                append("${entry.username}@${entry.host}:${entry.port}\n${entry.group} · ${entry.authMethod}")
                                if (entry.privateKeyPath.isNotBlank()) append("\n${entry.privateKeyPath}")
                                if (entry.certificatePath.isNotBlank()) append("\n${entry.certificatePath}")
                                if (entry.startupCommand.isNotBlank()) append("\n${entry.startupCommand}")
                                append("\n" + stringResource(R.string.settings_import_credentials,
                                    stringResource(if (entry.hasPassword) R.string.settings_yes else R.string.settings_no),
                                    stringResource(if (entry.hasPassphrase) R.string.settings_yes else R.string.settings_no),
                                    stringResource(if (entry.agentForwarding) R.string.settings_yes else R.string.settings_no)))
                            })
                        })
                        HorizontalDivider()
                    }
                    itemsIndexed(preview.warnings) { _, warning ->
                        SettingsErrorText("${warning.entryName}: ${warning.reason}")
                    }
                if (!operations.previewValid && !busy) {
                    item {
                    Text(stringResource(R.string.settings_preview_expired))
                    OutlinedButton(enabled = operations.uri != null, onClick = {
                        operations.uri?.let { operations.readImport(repository, context.applicationContext.contentResolver, it) }
                    }) { Text(stringResource(R.string.settings_reread_file)) }
                    }
                }
            }
        }
        if (count != null) Button(onClick = { operations.closeImport(repository) }) { Text(stringResource(R.string.settings_done)) }
        else if (preview != null && operations.previewValid) Button(enabled = !busy && preview.entries.isNotEmpty(), onClick = { confirm = true }, modifier = Modifier.fillMaxWidth()) {
            Text(stringResource(R.string.settings_confirm_batch, preview.entries.size))
        }
    }
    if (confirm) AlertDialog(onDismissRequest = { confirm = false }, title = { Text(stringResource(R.string.settings_confirm_batch, preview?.entries?.size ?: 0)) },
        text = { Text(stringResource(R.string.settings_import_all)) },
        confirmButton = { TextButton(onClick = { confirm = false; operations.commitImport(repository) }) { Text(stringResource(R.string.keychain_source_imported)) } },
        dismissButton = { TextButton(onClick = { confirm = false }) { Text(stringResource(R.string.tool_action_cancel)) } })
}
