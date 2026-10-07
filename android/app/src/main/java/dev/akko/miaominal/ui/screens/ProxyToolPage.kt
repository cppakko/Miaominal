package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.ui.uiString

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Hub
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Badge
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.layout.heightIn
import androidx.compose.ui.semantics.Role
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.ui.components.PageEmptyState
import dev.akko.miaominal.R
import dev.akko.miaominal.data.CredentialHealth
import dev.akko.miaominal.data.CredentialUpdate
import dev.akko.miaominal.data.ProxyProfile
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.PageSubtitle
import dev.akko.miaominal.ui.components.SectionTitle
import kotlinx.coroutines.launch

/**
 * Reusable entry-proxy management. Proxies saved here can be selected in the
 * host editor and are used by SSH, SFTP, forwarding, and key deployment.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ProxyToolPage(modifier: Modifier = Modifier) {
    val repository = LocalMiaominalRepository.current
    val scope = rememberCoroutineScope()
    val proxies by repository.proxies.collectAsState()
    var editorVisible by rememberSaveable { mutableStateOf(false) }
    var editingId by rememberSaveable { mutableStateOf<String?>(null) }
    var name by rememberSaveable { mutableStateOf("") }
    var protocol by rememberSaveable { mutableStateOf("socks5") }
    var host by rememberSaveable { mutableStateOf("") }
    var port by rememberSaveable { mutableStateOf("1080") }
    var authMode by rememberSaveable { mutableStateOf("none") }
    var username by rememberSaveable { mutableStateOf("") }
    var password by rememberSaveable { mutableStateOf("") }
    var clearPassword by rememberSaveable { mutableStateOf(false) }
    var resolveDns by rememberSaveable { mutableStateOf(false) }
    var validationError by rememberSaveable { mutableStateOf(false) }
    var saveError by rememberSaveable { mutableStateOf<String?>(null) }

    fun openNew() {
        editingId = null
        name = ""
        protocol = "socks5"
        host = ""
        port = "1080"
        authMode = "none"
        username = ""
        password = ""
        clearPassword = false
        resolveDns = false
        validationError = false
        saveError = null
        editorVisible = true
    }

    fun openEdit(proxy: ProxyProfile) {
        editingId = proxy.id
        name = proxy.name
        protocol = proxy.protocol
        host = proxy.host
        port = proxy.port.toString()
        authMode = proxy.authMode
        username = proxy.username
        password = ""
        clearPassword = false
        resolveDns = proxy.resolveDnsThroughProxy
        validationError = false
        saveError = null
        editorVisible = true
    }

    if (editorVisible) {
        AlertDialog(
            onDismissRequest = { editorVisible = false },
            title = {
                Text(
                    stringResource(
                        if (editingId == null) R.string.proxy_add else R.string.proxy_edit
                    )
                )
            },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    OutlinedTextField(
                        value = name,
                        onValueChange = { name = it },
                        label = { Text(stringResource(R.string.proxy_name)) },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth()
                    )
                    Text(
                        stringResource(R.string.proxy_protocol),
                        style = MaterialTheme.typography.labelLarge
                    )
                    SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                        listOf("socks5" to "SOCKS5", "http-connect" to "HTTP CONNECT")
                            .forEachIndexed { index, (value, label) ->
                                SegmentedButton(
                                    selected = protocol == value,
                                    onClick = { protocol = value },
                                    shape = SegmentedButtonDefaults.itemShape(
                                        index = index,
                                        count = 2
                                    )
                                ) {
                                    Text(label)
                                }
                            }
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedTextField(
                            value = host,
                            onValueChange = { host = it },
                            label = { Text(stringResource(R.string.proxy_host)) },
                            singleLine = true,
                            modifier = Modifier.weight(2f)
                        )
                        OutlinedTextField(
                            value = port,
                            onValueChange = { port = it.filter(Char::isDigit).take(5) },
                            label = { Text(stringResource(R.string.proxy_port)) },
                            singleLine = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                            modifier = Modifier.weight(1f)
                        )
                    }
                    Text(
                        stringResource(R.string.proxy_password),
                        style = MaterialTheme.typography.labelLarge
                    )
                    SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                        listOf("none" to R.string.proxy_auth_none, "username-password" to R.string.proxy_auth_password)
                            .forEachIndexed { index, (value, label) ->
                                SegmentedButton(
                                    selected = authMode == value,
                                    onClick = { authMode = value },
                                    shape = SegmentedButtonDefaults.itemShape(
                                        index = index,
                                        count = 2
                                    )
                                ) {
                                    Text(stringResource(label))
                                }
                            }
                    }
                    if (authMode == "username-password") {
                        OutlinedTextField(
                            value = username,
                            onValueChange = { username = it },
                            label = { Text(stringResource(R.string.proxy_username)) },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth()
                        )
                        OutlinedTextField(
                            value = password,
                            onValueChange = {
                                password = it
                                clearPassword = false
                            },
                            label = { Text(stringResource(R.string.proxy_password)) },
                            singleLine = true,
                            visualTransformation = PasswordVisualTransformation(),
                            supportingText = {
                                Text(
                                    if (editingId != null) {
                                        stringResource(R.string.proxy_password_stored)
                                    } else {
                                        stringResource(R.string.proxy_password_missing)
                                    }
                                )
                            },
                            modifier = Modifier.fillMaxWidth()
                        )
                        if (editingId != null) {
                            TextButton(onClick = {
                                clearPassword = true
                                password = ""
                            }) {
                                Text(stringResource(R.string.proxy_password_clear))
                            }
                        }
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth().toggleable(
                            value = resolveDns, role = Role.Switch, onValueChange = { resolveDns = it }
                        ).heightIn(min = 48.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Switch(checked = resolveDns, onCheckedChange = null)
                        Spacer(Modifier.width(8.dp))
                        Text(stringResource(R.string.proxy_resolve_dns))
                    }
                    if (validationError) {
                        Text(
                            text = stringResource(R.string.tool_form_required),
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodyMedium
                        )
                    }
                    saveError?.let { message ->
                        Text(
                            text = message,
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodyMedium
                        )
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    val valid = name.isNotBlank() && host.isNotBlank() &&
                        port.toIntOrNull() != null &&
                        (authMode == "none" || username.isNotBlank())
                    validationError = !valid
                    if (valid) {
                        val proxy = ProxyProfile(
                            id = editingId.orEmpty(),
                            name = name.trim(),
                            protocol = protocol,
                            host = host.trim(),
                            port = port.toIntOrNull() ?: 0,
                            authMode = authMode,
                            username = username.trim(),
                            resolveDnsThroughProxy = resolveDns
                        )
                        val action = when {
                            clearPassword -> CredentialUpdate.Clear
                            password.isNotEmpty() -> CredentialUpdate.Set(password)
                            else -> CredentialUpdate.Keep
                        }
                        scope.launch {
                            val result = repository.saveProxy(proxy, action)
                            if (result.isSuccess) {
                                editorVisible = false
                            } else {
                                saveError = result.exceptionOrNull()?.message ?: uiString(R.string.ui_could_not_save)
                            }
                        }
                    }
                }) {
                    Text(stringResource(R.string.tool_action_save))
                }
            },
            dismissButton = {
                TextButton(onClick = { editorVisible = false }) {
                    Text(stringResource(R.string.tool_action_cancel))
                }
            }
        )
    }

    Column(modifier = modifier.fillMaxSize()) {
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(16.dp, 16.dp, 16.dp, 96.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            item {
                PageSubtitle(text = stringResource(R.string.tool_proxies_desc))
            }
            if (proxies.isEmpty()) {
                item {
                    PageEmptyState(
                        title = stringResource(R.string.page_empty_title),
                        modifier = Modifier.fillParentMaxHeight(0.65f),
                        icon = Icons.Outlined.Hub,
                        text = stringResource(R.string.proxy_empty)
                    )
                }
            } else {
                items(proxies, key = { it.id }) { proxy ->
                    ProxyCard(
                        proxy = proxy,
                        onEdit = { openEdit(proxy) },
                        onDelete = {
                            scope.launch { repository.deleteProxy(proxy.id) }
                        }
                    )
                }
            }
        }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            horizontalArrangement = Arrangement.End
        ) {
            FloatingActionButton(
                onClick = ::openNew,
                containerColor = MaterialTheme.colorScheme.primaryContainer,
                contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                shape = MaterialTheme.shapes.large
            ) {
                Icon(
                    imageVector = Icons.Outlined.Add,
                    contentDescription = stringResource(R.string.proxy_add)
                )
            }
        }
    }
}

@Composable
private fun ProxyCard(
    proxy: ProxyProfile,
    onEdit: () -> Unit,
    onDelete: () -> Unit
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
        )
    ) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            SectionTitle(title = proxy.name)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Badge(
                    containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                    contentColor = MaterialTheme.colorScheme.onSurfaceVariant
                ) {
                    Text(
                        text = if (proxy.protocol == "http-connect") "HTTP CONNECT" else "SOCKS5",
                        modifier = Modifier.padding(horizontal = 4.dp),
                        style = MaterialTheme.typography.labelMedium
                    )
                }
                Badge(
                    containerColor = when {
                        proxy.authMode != "username-password" ->
                            MaterialTheme.colorScheme.surfaceContainerHighest
                        proxy.hasStoredPassword -> MaterialTheme.colorScheme.primaryContainer
                        else -> MaterialTheme.colorScheme.errorContainer
                    },
                    contentColor = when {
                        proxy.authMode != "username-password" ->
                            MaterialTheme.colorScheme.onSurfaceVariant
                        proxy.hasStoredPassword -> MaterialTheme.colorScheme.onPrimaryContainer
                        else -> MaterialTheme.colorScheme.onErrorContainer
                    }
                ) {
                    Text(
                        text = if (proxy.authMode == "username-password") {
                            if (proxy.hasStoredPassword) {
                                stringResource(R.string.proxy_auth_password)
                            } else {
                                stringResource(R.string.proxy_password_missing)
                            }
                        } else {
                            stringResource(R.string.proxy_auth_none)
                        },
                        modifier = Modifier.padding(horizontal = 4.dp),
                        style = MaterialTheme.typography.labelMedium
                    )
                }
            }
            Text(
                text = "${proxy.host}:${proxy.port}",
                style = MaterialTheme.typography.bodyMedium
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End
            ) {
                Button(onClick = onEdit, colors = ButtonDefaults.textButtonColors()) {
                    Icon(Icons.Outlined.Edit, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.tool_action_edit))
                }
                OutlinedButton(
                    onClick = onDelete,
                    colors = ButtonDefaults.outlinedButtonColors(
                        contentColor = MaterialTheme.colorScheme.error
                    )
                ) {
                    Icon(Icons.Outlined.Delete, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.proxy_delete))
                }
            }
        }
    }
}
