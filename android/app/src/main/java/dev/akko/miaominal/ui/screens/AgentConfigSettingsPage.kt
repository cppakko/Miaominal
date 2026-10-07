package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.data.CredentialUpdate
import dev.akko.miaominal.data.agent.*
import dev.akko.miaominal.ui.LocalAgentConfigRepository
import kotlinx.coroutines.launch

@Composable
fun AgentConfigSettingsPage(
    modifier: Modifier = Modifier,
    repository: AgentConfigRepository? = LocalAgentConfigRepository.current
) {
    if (repository == null) {
        Text(stringResource(R.string.agent_config_unavailable), modifier.padding(20.dp))
        return
    }
    val state by repository.state.collectAsState()
    val scope = rememberCoroutineScope()
    var editing by remember { mutableStateOf<AgentConfigProfile?>(null) }
    var editorOpen by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf<AgentConfigProfile?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(repository) { repository.load() }

    LazyColumn(modifier.testTag("agent-config-settings"), contentPadding = PaddingValues(20.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item(key = "ai-configurations") {
            Text(stringResource(R.string.agent_config_description), style = MaterialTheme.typography.bodyMedium)
            Spacer(Modifier.height(12.dp))
            Button(onClick = { editing = null; editorOpen = true },
                enabled = state.loaded && state.error == null,
                modifier = Modifier.testTag("agent-config-add")) {
                Icon(Icons.Outlined.Add, null)
                Spacer(Modifier.width(8.dp))
                Text(stringResource(R.string.agent_config_add))
            }
        }
        if (!state.loaded) item { CircularProgressIndicator() }
        val failure = state.error ?: error
        if (failure != null) item {
            Text(failure, color = MaterialTheme.colorScheme.error)
            TextButton(onClick = { scope.launch { error = null; repository.load() } }) {
                Text(stringResource(R.string.agent_config_retry))
            }
        }
        items(state.profiles, key = { it.id }) { profile ->
            OutlinedCard(modifier = Modifier.fillMaxWidth().testTag("agent-config-${profile.id}"),
                onClick = { editing = profile; editorOpen = true }) {
                Row(Modifier.fillMaxWidth().padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(profile.name, style = MaterialTheme.typography.titleMedium)
                        Text(profile.baseUrl, style = MaterialTheme.typography.bodySmall)
                        Text("${protocolLabel(profile.protocol)} · ${profile.defaultModel}", style = MaterialTheme.typography.bodySmall)
                        if (!profile.hasApiKey) Text(stringResource(R.string.agent_config_missing_key),
                            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                    }
                    IconButton(onClick = { deleting = profile }) {
                        Icon(Icons.Outlined.Delete, stringResource(R.string.agent_config_delete))
                    }
                }
            }
        }
    }
    if (editorOpen) AgentConfigEditor(editing, repository, onDismiss = { editorOpen = false })
    deleting?.let { profile ->
        AlertDialog(onDismissRequest = { deleting = null },
            title = { Text(stringResource(R.string.agent_config_delete)) },
            text = { Text(stringResource(R.string.agent_config_delete_detail, profile.name)) },
            confirmButton = { TextButton(onClick = { scope.launch {
                repository.delete(profile.id).onFailure { error = it.message }
                deleting = null
            } }) { Text(stringResource(R.string.agent_config_delete)) } },
            dismissButton = { TextButton(onClick = { deleting = null }) { Text(stringResource(R.string.agent_config_cancel)) } })
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AgentConfigEditor(profile: AgentConfigProfile?, repository: AgentConfigRepository, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    val id = remember(profile?.id) { profile?.id ?: AgentConfigRepository.newId() }
    var name by remember(id) { mutableStateOf(profile?.name.orEmpty()) }
    var url by remember(id) { mutableStateOf(profile?.baseUrl.orEmpty()) }
    var protocol by remember(id) { mutableStateOf(profile?.protocol ?: AgentApiProtocol.Responses) }
    var models by remember(id) { mutableStateOf(profile?.models.orEmpty()) }
    var defaultModel by remember(id) { mutableStateOf(profile?.defaultModel.orEmpty()) }
    var modelInput by remember(id) { mutableStateOf("") }
    // Credentials are transient, never rememberSaveable and never prefilled.
    var key by remember(id) { mutableStateOf("") }
    var clearKey by remember(id) { mutableStateOf(false) }
    var saving by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    ModalBottomSheet(onDismissRequest = { if (!saving) onDismiss() },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().imePadding().verticalScroll(rememberScrollState()).padding(20.dp)
            .testTag("agent-config-editor"), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.agent_config_title), style = MaterialTheme.typography.titleLarge)
            OutlinedTextField(name, { name = it }, enabled = !saving, singleLine = true,
                label = { Text(stringResource(R.string.agent_config_name)) }, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(url, { url = it }, enabled = !saving, singleLine = true,
                label = { Text("Base URL") }, supportingText = { Text(stringResource(R.string.agent_config_remote_url)) },
                modifier = Modifier.fillMaxWidth())
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                AgentApiProtocol.entries.forEach { entry ->
                    FilterChip(selected = protocol == entry, enabled = !saving, onClick = { protocol = entry },
                        label = { Text(protocolLabel(entry)) })
                }
            }
            OutlinedTextField(key, { key = it; clearKey = false }, enabled = !saving, singleLine = true,
                visualTransformation = PasswordVisualTransformation(), label = { Text("API Key") },
                supportingText = { Text(stringResource(if (profile?.hasApiKey == true && !clearKey)
                    R.string.agent_config_key_keep else R.string.agent_config_key_hint)) },
                modifier = Modifier.fillMaxWidth())
            if (profile?.hasApiKey == true) Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(clearKey, { clearKey = it; if (it) key = "" }, enabled = !saving)
                Text(stringResource(R.string.agent_config_clear_key))
            }
            Text(stringResource(R.string.agent_config_models), style = MaterialTheme.typography.titleSmall)
            models.forEach { model ->
                Row(Modifier.fillMaxWidth().clickable(enabled = !saving) { defaultModel = model },
                    verticalAlignment = Alignment.CenterVertically) {
                    RadioButton(defaultModel == model, { defaultModel = model }, enabled = !saving)
                    Text(model, Modifier.weight(1f))
                    IconButton(enabled = !saving, onClick = {
                        models = models - model
                        if (defaultModel == model) defaultModel = models.firstOrNull().orEmpty()
                    }) { Icon(Icons.Outlined.Delete, stringResource(R.string.agent_config_remove_model)) }
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(modelInput, { modelInput = it }, enabled = !saving, singleLine = true,
                    label = { Text(stringResource(R.string.agent_config_model_id)) }, modifier = Modifier.weight(1f))
                IconButton(enabled = !saving && modelInput.isNotBlank(), onClick = {
                    val model = modelInput.trim()
                    models = (models + model).distinct()
                    if (defaultModel.isBlank()) defaultModel = model
                    modelInput = ""
                }) { Icon(Icons.Outlined.Add, stringResource(R.string.agent_config_add_model)) }
            }
            Text(stringResource(R.string.agent_config_default_hint), style = MaterialTheme.typography.bodySmall)
            error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            Button(enabled = !saving, modifier = Modifier.fillMaxWidth().testTag("agent-config-save"), onClick = {
                saving = true
                error = null
                scope.launch {
                    val credential = when {
                        clearKey -> CredentialUpdate.Clear
                        key.isNotBlank() -> CredentialUpdate.Set(key)
                        else -> CredentialUpdate.Keep
                    }
                    repository.save(AgentConfigProfile(id, name, protocol, url, models, defaultModel), credential)
                        .onSuccess { key = ""; onDismiss() }.onFailure { error = it.message }
                    saving = false
                }
            }) { Text(stringResource(R.string.agent_config_save)) }
            Text(stringResource(R.string.agent_config_next_connection), style = MaterialTheme.typography.bodySmall)
        }
    }
}

@Composable
internal fun AgentConfigSelector(
    profiles: List<AgentConfigProfile>, source: AgentConfigSource, enabled: Boolean,
    onSelect: (AgentConfigSource) -> Unit, onManage: () -> Unit
) {
    var expanded by remember { mutableStateOf(false) }
    val label = when (source) {
        AgentConfigSource.Remote -> stringResource(R.string.agent_config_remote)
        is AgentConfigSource.Local -> profiles.firstOrNull { it.id == source.profileId }?.name
            ?: stringResource(R.string.agent_config_missing)
    }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f)) {
            OutlinedButton(onClick = { expanded = true }, enabled = enabled,
                modifier = Modifier.fillMaxWidth().testTag("agent-config-selector")) { Text(label) }
            DropdownMenu(expanded, { expanded = false }) {
                DropdownMenuItem(text = { Text(stringResource(R.string.agent_config_remote)) }, onClick = {
                    expanded = false; onSelect(AgentConfigSource.Remote)
                })
                profiles.forEach { profile ->
                    DropdownMenuItem(text = { Column {
                        Text(profile.name)
                        if (!profile.hasApiKey) Text(stringResource(R.string.agent_config_missing_key),
                            style = MaterialTheme.typography.bodySmall)
                    } }, enabled = profile.hasApiKey, onClick = {
                        expanded = false; onSelect(AgentConfigSource.Local(profile.id))
                    })
                }
            }
        }
        TextButton(onClick = onManage, enabled = enabled) { Text(stringResource(R.string.agent_config_manage)) }
    }
}

@Composable
private fun protocolLabel(protocol: AgentApiProtocol) = when (protocol) {
    AgentApiProtocol.Responses -> "Responses"
    AgentApiProtocol.ChatCompletions -> "Chat Completions"
}
