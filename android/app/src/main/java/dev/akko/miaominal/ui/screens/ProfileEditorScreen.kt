package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.ui.theme.UiSpacing
import androidx.compose.foundation.layout.size

import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.foundation.layout.heightIn
import dev.akko.miaominal.ui.model.EditorErrorField
import dev.akko.miaominal.ui.model.firstEditorError

import dev.akko.miaominal.ui.uiString

import dev.akko.miaominal.ui.components.WorkspaceBackHandler as BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.StarBorder
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Badge
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.FilterChip
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuAnchorType
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import kotlinx.coroutines.launch
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.data.ManagedKeyRecord
import dev.akko.miaominal.data.SessionEnvironmentVariable
import dev.akko.miaominal.data.ShellType
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.SectionTitle
import dev.akko.miaominal.ui.components.JumpChainEditor
import dev.akko.miaominal.ui.model.AuthMethodUi
import dev.akko.miaominal.ui.model.HostProfileUi
import dev.akko.miaominal.ui.model.ProfileKindUi
import dev.akko.miaominal.ui.model.parseTags

private data class ProxyOption(val id: String, val label: String)

const val ProfileShellTypeTag = "profile-shell-type"
const val ProfileEnvironmentAddTag = "profile-environment-add"
const val ProfileSaveTag = "profile-save"
fun profileShellOptionTag(shellType: ShellType) = "profile-shell-${shellType.name}"
fun profileEnvironmentNameTag(index: Int) = "profile-environment-name-$index"
fun profileEnvironmentValueTag(index: Int) = "profile-environment-value-$index"

internal fun normalizeEnvironmentVariables(
    variables: List<SessionEnvironmentVariable>
): Result<List<SessionEnvironmentVariable>> = runCatching {
    variables.mapIndexedNotNull { index, variable ->
        val name = variable.name.trim()
        if (name.isEmpty() && variable.value.isEmpty()) return@mapIndexedNotNull null
        require(name.isNotEmpty()) { "environment variable ${index + 1} has no name" }
        require(ENVIRONMENT_NAME.matches(name)) { "invalid environment variable name: $name" }
        SessionEnvironmentVariable(name = name, value = variable.value)
    }
}

private val ENVIRONMENT_NAME = Regex("[A-Za-z_][A-Za-z0-9_]*")

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun ProfileEditorScreen(
    initial: HostProfileUi?,
    availableGroups: List<String>,
    managedKeys: List<ManagedKeyRecord> = emptyList(),
    hasStoredPassword: Boolean = false,
    onSave: suspend (HostProfileUi, String?) -> Result<Unit>,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier
) {
    val scope = rememberCoroutineScope()
    val repository = LocalMiaominalRepository.current
    val proxies by repository.proxies.collectAsState()
    val allHosts by repository.hosts.collectAsState()
    val editorListState = rememberLazyListState()
    var editorViewport by remember { mutableStateOf(Rect.Zero) }
    var saveError by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    var name by rememberSaveable { mutableStateOf(initial?.name ?: "") }
    var host by rememberSaveable { mutableStateOf(initial?.address ?: "") }
    var port by rememberSaveable { mutableStateOf(initial?.port?.toString() ?: "22") }
    var username by rememberSaveable { mutableStateOf(initial?.username ?: "") }
    var group by rememberSaveable { mutableStateOf(initial?.group ?: "") }
    var creatingNewGroup by rememberSaveable { mutableStateOf(false) }
    var groupMenuExpanded by rememberSaveable { mutableStateOf(false) }
    var tagsText by rememberSaveable { mutableStateOf(initial?.tags?.joinToString(", ") ?: "") }
    val kind = initial?.kind ?: ProfileKindUi.Ssh
    var authMethod by rememberSaveable { mutableStateOf(initial?.authMethod ?: AuthMethodUi.Password) }
    var privateKeyPath by rememberSaveable { mutableStateOf(initial?.privateKeyPath ?: "") }
    var managedKeyId by rememberSaveable { mutableStateOf(initial?.managedKeyId ?: "") }
    var managedKeyMenuExpanded by rememberSaveable { mutableStateOf(false) }
    var agentIdentity by rememberSaveable { mutableStateOf(initial?.agentIdentity ?: "") }
    var startupCommand by rememberSaveable { mutableStateOf(initial?.startupCommand ?: "") }
    var shellTypeName by rememberSaveable { mutableStateOf((initial?.shellType ?: ShellType.Posix).name) }
    var environmentFields by rememberSaveable {
        mutableStateOf(
            ArrayList(
                (initial?.environmentVariables ?: emptyList()).flatMap { variable ->
                    listOf(variable.name, variable.value)
                }
            )
        )
    }
    var isFavorite by rememberSaveable { mutableStateOf(initial?.isFavorite ?: false) }
    var password by rememberSaveable { mutableStateOf("") }
    var passwordVisible by rememberSaveable { mutableStateOf(false) }
    var clearStoredPassword by rememberSaveable { mutableStateOf(false) }
    var showValidation by rememberSaveable { mutableStateOf(false) }
    var discardDialogVisible by rememberSaveable { mutableStateOf(false) }
    var entryProxyId by rememberSaveable { mutableStateOf(initial?.entryProxyId ?: "") }
    var jumpChainCsv by rememberSaveable {
        mutableStateOf(initial?.proxyJumpProfileIds?.joinToString(",") ?: "")
    }
    val jumpChain = remember(jumpChainCsv) {
        jumpChainCsv.split(',').map { it.trim() }.filter { it.isNotEmpty() }
    }
    val shellType = ShellType.valueOf(shellTypeName)
    val environmentVariables = environmentFields.chunked(2).map { fields ->
        SessionEnvironmentVariable(
            name = fields.getOrElse(0) { "" },
            value = fields.getOrElse(1) { "" }
        )
    }
    val normalizedEnvironment = normalizeEnvironmentVariables(environmentVariables)

    val initialSnapshot = remember {
        listOf(
            initial?.name ?: "",
            initial?.address ?: "",
            initial?.port?.toString() ?: "22",
            initial?.username ?: "",
            initial?.group ?: "",
            initial?.tags?.joinToString(", ") ?: "",
            (initial?.kind ?: ProfileKindUi.Ssh).name,
            (initial?.authMethod ?: AuthMethodUi.Password).name,
            initial?.privateKeyPath ?: "",
            initial?.managedKeyId ?: "",
            initial?.agentIdentity ?: "",
            initial?.startupCommand ?: "",
            (initial?.shellType ?: ShellType.Posix).name,
            (initial?.environmentVariables ?: emptyList()).joinToString("\u0000") {
                "${it.name}\u0001${it.value}"
            },
            (initial?.isFavorite ?: false).toString(),
            initial?.entryProxyId ?: "",
            initial?.proxyJumpProfileIds?.joinToString(",") ?: ""
        )
    }
    val currentSnapshot = listOf(
        name,
        host,
        port,
        username,
        group,
        tagsText,
        kind.name,
        authMethod.name,
        privateKeyPath,
        managedKeyId,
        agentIdentity,
        startupCommand,
        shellTypeName,
        environmentVariables.joinToString("\u0000") { "${it.name}\u0001${it.value}" },
        isFavorite.toString(),
        entryProxyId,
        jumpChainCsv
    )
    val dirty = currentSnapshot != initialSnapshot || password.isNotEmpty() || clearStoredPassword
    val tags = remember(tagsText) { parseTags(tagsText) }
    val portValue = port.toIntOrNull()
    val portValid = portValue != null && portValue in 1..65535
    val namedFieldsValid = name.isNotBlank() && host.isNotBlank()
    val snackbar = remember { SnackbarHostState() }
    val nameFocus = remember { FocusRequester() }
    val hostFocus = remember { FocusRequester() }
    val portFocus = remember { FocusRequester() }
    val environmentFocus = remember { FocusRequester() }
    var validationTarget by remember { mutableStateOf<EditorErrorField?>(null) }
    var validationRevision by remember { mutableStateOf(0) }
    LaunchedEffect(validationRevision) {
        val target = validationTarget ?: return@LaunchedEffect
        androidx.compose.runtime.withFrameNanos { }
        val key = when (target) {
            EditorErrorField.Name -> "name"
            EditorErrorField.Host -> "host"
            EditorErrorField.Port -> "port"
            EditorErrorField.Environment -> "environment"
        }
        // Basic field indices are stable; environment fields are located by their lazy item key.
        if (target != EditorErrorField.Environment) editorListState.animateScrollToItem(
            when (target) { EditorErrorField.Name -> 1; EditorErrorField.Host -> 2; else -> 3 } + if (saveError != null) 1 else 0)
        else {
            // Locate a lazily composed field without maintaining indices for conditional auth sections.
            var index = 0
            while (index < editorListState.layoutInfo.totalItemsCount) {
                editorListState.scrollToItem(index)
                if (editorListState.layoutInfo.visibleItemsInfo.any { it.key == key }) break
                index += 1
            }
        }
        androidx.compose.runtime.withFrameNanos { }
        runCatching { when (target) {
            EditorErrorField.Name -> nameFocus
            EditorErrorField.Host -> hostFocus
            EditorErrorField.Port -> portFocus
            EditorErrorField.Environment -> environmentFocus
        }.requestFocus() }
    }
    LaunchedEffect(saveError) { saveError?.let { snackbar.showSnackbar(it) } }
    val selectedKeyName = managedKeys.firstOrNull { it.id == managedKeyId }?.name ?: ""

    fun requestClose() {
        if (dirty) discardDialogVisible = true else onDismiss()
    }

    fun submit() {
        showValidation = true
        if (saving) return
        validationTarget = firstEditorError(name, host, port, normalizedEnvironment.isSuccess)
        if (validationTarget != null) { validationRevision += 1; return }
        val resolvedPort = portValue ?: 22
        val passwordToSubmit = when {
            authMethod != AuthMethodUi.Password -> null
            password.isNotEmpty() -> password
            clearStoredPassword -> ""
            else -> null
        }
        val profile = HostProfileUi(
            id = initial?.id ?: "host-${System.currentTimeMillis()}",
            name = name.trim(),
            address = host.trim(),
            username = username.trim(),
            group = group.trim(),
            tags = tags,
            port = resolvedPort,
            kind = kind,
            authMethod = authMethod,
            privateKeyPath = privateKeyPath.trim(),
            managedKeyId = managedKeyId,
            agentIdentity = agentIdentity.trim(),
            startupCommand = startupCommand.trim(),
            environmentVariables = normalizedEnvironment.getOrThrow(),
            shellType = shellType,
            isFavorite = isFavorite,
            latency = initial?.latency ?: "--",
            lastConnected = initial?.lastConnected ?: "",
            online = initial?.online ?: false,
            entryProxyId = entryProxyId,
            proxyJumpProfileIds = jumpChain
        )
        saving = true
        saveError = null
        scope.launch {
            val result = onSave(profile, passwordToSubmit)
            saving = false
            if (result.isFailure) {
                saveError = result.exceptionOrNull()?.let { error ->
                    error.message?.takeIf(String::isNotBlank) ?: error.javaClass.simpleName
                } ?: uiString(R.string.ui_could_not_save)
            }
        }
    }

    BackHandler(onBack = ::requestClose)

    Scaffold(
        modifier = modifier,
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        stringResource(
                            if (initial == null) R.string.profile_new_title else R.string.profile_edit_title
                        )
                    )
                },
                navigationIcon = {
                    IconButton(onClick = ::requestClose) {
                        Icon(
                            imageVector = Icons.Outlined.Close,
                            contentDescription = stringResource(R.string.profile_close)
                        )
                    }
                },
                actions = {
                    IconButton(onClick = { isFavorite = !isFavorite }) {
                        Icon(
                            imageVector = if (isFavorite) Icons.Filled.Star else Icons.Outlined.StarBorder,
                            contentDescription = stringResource(R.string.profile_favorite)
                        )
                    }
                    TextButton(
                        onClick = ::submit,
                        enabled = !saving,
                        modifier = Modifier.testTag(ProfileSaveTag)
                    ) {
                        if (saving) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                        Text(stringResource(if (saving) R.string.ux_saving else R.string.profile_save))
                    }
                }
            )
        }
    ) { contentPadding ->
        LazyColumn(
            state = editorListState,
            modifier = Modifier
                .fillMaxWidth()
                .padding(contentPadding)
                .wrapContentWidth()
                .widthIn(max = 720.dp)
                .fillMaxWidth()
                .onGloballyPositioned { editorViewport = it.boundsInRoot() },
            contentPadding = PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            if (saveError != null) {
                item {
                    Text(
                        text = stringResource(R.string.profile_save_failed, saveError ?: ""),
                        color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodyMedium
                    )
                }
            }
            item {
                SectionTitle(title = stringResource(R.string.profile_section_basic))
            }
            item(key = "name") {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    modifier = Modifier.fillMaxWidth().focusRequester(nameFocus),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next),
                    singleLine = true,
                    label = { Text(stringResource(R.string.profile_name)) },
                    placeholder = { Text(stringResource(R.string.profile_name_ph)) },
                    isError = showValidation && name.isBlank(),
                    supportingText = if (showValidation && name.isBlank()) {
                        { Text(stringResource(R.string.profile_required)) }
                    } else {
                        null
                    }
                )
            }
            item(key = "host") {
                OutlinedTextField(
                    value = host,
                    onValueChange = { host = it },
                    modifier = Modifier.fillMaxWidth().focusRequester(hostFocus),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next),
                    singleLine = true,
                    label = { Text(stringResource(R.string.profile_host)) },
                    placeholder = { Text(stringResource(R.string.profile_host_ph)) },
                    isError = showValidation && host.isBlank(),
                    supportingText = if (showValidation && host.isBlank()) { { Text(stringResource(R.string.profile_required)) } } else null
                )
            }
            item {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    OutlinedTextField(
                        value = port,
                        onValueChange = { port = it.filter(Char::isDigit).take(5) },
                        modifier = Modifier.weight(1f).focusRequester(portFocus),
                        singleLine = true,
                        label = { Text(stringResource(R.string.profile_port)) },
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, imeAction = ImeAction.Next),
                        isError = showValidation && !portValid,
                        supportingText = if (showValidation && !portValid) {
                            { Text(stringResource(R.string.profile_port_error)) }
                        } else {
                            null
                        }
                    )
                    OutlinedTextField(
                        value = username,
                        onValueChange = { username = it },
                        modifier = Modifier.weight(2f),
                        singleLine = true,
                        label = { Text(stringResource(R.string.profile_username)) },
                        placeholder = { Text(stringResource(R.string.profile_username_ph)) }
                    )
                }
            }

            item {
                SectionTitle(title = stringResource(R.string.profile_section_category))
            }
            item {
                ExposedDropdownMenuBox(
                    expanded = groupMenuExpanded,
                    onExpandedChange = { groupMenuExpanded = it }
                ) {
                    OutlinedTextField(
                        value = group,
                        onValueChange = { group = it },
                        modifier = Modifier
                            .fillMaxWidth()
                            .menuAnchor(MenuAnchorType.PrimaryNotEditable, enabled = !creatingNewGroup),
                        readOnly = !creatingNewGroup,
                        singleLine = true,
                        label = { Text(stringResource(R.string.profile_group)) },
                        placeholder = {
                            if (creatingNewGroup) {
                                Text(stringResource(R.string.profile_group_new_ph))
                            }
                        },
                        trailingIcon = {
                            ExposedDropdownMenuDefaults.TrailingIcon(expanded = groupMenuExpanded)
                        }
                    )
                    ExposedDropdownMenu(
                        expanded = groupMenuExpanded,
                        onDismissRequest = { groupMenuExpanded = false }
                    ) {
                        availableGroups.forEach { candidate ->
                            DropdownMenuItem(
                                text = { Text(candidate) },
                                onClick = {
                                    group = candidate
                                    creatingNewGroup = false
                                    groupMenuExpanded = false
                                }
                            )
                        }
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.profile_new_group)) },
                            onClick = {
                                creatingNewGroup = true
                                group = ""
                                groupMenuExpanded = false
                            }
                        )
                    }
                }
            }
            item {
                Column(verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                    OutlinedTextField(
                        value = tagsText,
                        onValueChange = { tagsText = it },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        label = { Text(stringResource(R.string.profile_tags)) },
                        supportingText = { Text(stringResource(R.string.profile_tags_helper)) }
                    )
                    if (tags.isNotEmpty()) {
                        FlowRow(
                            horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)
                        ) {
                            tags.forEach { tag ->
                                Badge(
                                    modifier = Modifier.defaultMinSize(
                                        minWidth = 24.dp,
                                        minHeight = 24.dp
                                    ),
                                    containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                                    contentColor = MaterialTheme.colorScheme.onSurfaceVariant
                                ) {
                                    Text(
                                        text = tag,
                                        modifier = Modifier.padding(horizontal = UiSpacing.xxs),
                                        style = MaterialTheme.typography.labelMedium
                                    )
                                }
                            }
                        }
                    }
                }
            }

            item {
                SectionTitle(title = stringResource(R.string.profile_protocol_connection, stringResource(kind.labelRes)))
            }
            item {
                FlowRow(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)
                ) {
                    AuthMethodUi.entries.forEach { option ->
                        FilterChip(
                            selected = authMethod == option,
                            onClick = { authMethod = option },
                            label = { Text(stringResource(option.labelRes)) }
                        )
                    }
                }
            }
            if (authMethod == AuthMethodUi.Password) {
                item {
                    OutlinedTextField(
                        value = password,
                        onValueChange = {
                            password = it
                            if (it.isNotEmpty()) clearStoredPassword = false
                        },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        label = { Text(stringResource(R.string.profile_password)) },
                        placeholder = { Text(stringResource(R.string.profile_password_placeholder)) },
                        visualTransformation = if (passwordVisible) {
                            VisualTransformation.None
                        } else {
                            PasswordVisualTransformation()
                        },
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                        supportingText = when {
                            clearStoredPassword -> {
                                { Text(stringResource(R.string.profile_password_clear)) }
                            }
                            hasStoredPassword && password.isEmpty() -> {
                                { Text(stringResource(R.string.profile_password_hint)) }
                            }
                            else -> null
                        },
                        trailingIcon = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                if (hasStoredPassword && !clearStoredPassword) {
                                    IconButton(
                                        onClick = {
                                            password = ""
                                            clearStoredPassword = true
                                        }
                                    ) {
                                        Icon(
                                            imageVector = Icons.Outlined.DeleteOutline,
                                            contentDescription = stringResource(R.string.profile_password_clear)
                                        )
                                    }
                                }
                                IconButton(onClick = { passwordVisible = !passwordVisible }) {
                                    Icon(
                                        imageVector = if (passwordVisible) {
                                            Icons.Outlined.VisibilityOff
                                        } else {
                                            Icons.Outlined.Visibility
                                        },
                                        contentDescription = stringResource(
                                            if (passwordVisible) {
                                                R.string.profile_password_hide
                                            } else {
                                                R.string.profile_password_show
                                            }
                                        )
                                    )
                                }
                            }
                        }
                    )
                }
            }
            if (authMethod == AuthMethodUi.KeyFile) {
                item {
                    OutlinedTextField(
                        value = privateKeyPath,
                        onValueChange = { privateKeyPath = it },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        label = { Text(stringResource(R.string.profile_private_key)) },
                        placeholder = { Text(stringResource(R.string.profile_private_key_ph)) }
                    )
                }
            }
            if (authMethod == AuthMethodUi.ManagedKey) {
                item {
                    if (managedKeys.isEmpty()) {
                        Text(
                            text = stringResource(R.string.profile_managed_key_empty),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    } else {
                        ExposedDropdownMenuBox(
                            expanded = managedKeyMenuExpanded,
                            onExpandedChange = { managedKeyMenuExpanded = it }
                        ) {
                            OutlinedTextField(
                                value = selectedKeyName,
                                onValueChange = {},
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .menuAnchor(MenuAnchorType.PrimaryNotEditable),
                                readOnly = true,
                                singleLine = true,
                                label = { Text(stringResource(R.string.profile_managed_key)) },
                                placeholder = {
                                    Text(stringResource(R.string.profile_managed_key_placeholder))
                                },
                                trailingIcon = {
                                    ExposedDropdownMenuDefaults.TrailingIcon(
                                        expanded = managedKeyMenuExpanded
                                    )
                                }
                            )
                            ExposedDropdownMenu(
                                expanded = managedKeyMenuExpanded,
                                onDismissRequest = { managedKeyMenuExpanded = false }
                            ) {
                                managedKeys.forEach { key ->
                                    DropdownMenuItem(
                                        text = { Text("${key.name} · ${key.algorithm}") },
                                        onClick = {
                                            managedKeyId = key.id
                                            managedKeyMenuExpanded = false
                                        }
                                    )
                                }
                            }
                        }
                    }
                }
            }
            if (authMethod == AuthMethodUi.Agent) {
                item {
                    OutlinedTextField(
                        value = agentIdentity,
                        onValueChange = { agentIdentity = it },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        label = { Text(stringResource(R.string.profile_agent_identity)) },
                        placeholder = { Text(stringResource(R.string.profile_agent_identity_ph)) }
                    )
                }
            }

            item {
                SectionTitle(title = stringResource(R.string.forward_routing_group))
            }
            item {
                Column(verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                    val proxyOptions = remember(proxies) {
                        listOf(ProxyOption("", "")) + proxies.map {
                            ProxyOption(it.id, "${it.name} · ${it.host}:${it.port}")
                        }
                    }
                    val selectedProxy = proxyOptions.firstOrNull { it.id == entryProxyId }
                    var proxyMenuExpanded by rememberSaveable { mutableStateOf(false) }
                    ExposedDropdownMenuBox(
                        expanded = proxyMenuExpanded,
                        onExpandedChange = { proxyMenuExpanded = it }
                    ) {
                        OutlinedTextField(
                            value = selectedProxy?.label?.takeIf { it.isNotBlank() }
                                ?: stringResource(R.string.forward_entry_proxy_none),
                            onValueChange = {},
                            modifier = Modifier
                                .fillMaxWidth()
                                .menuAnchor(MenuAnchorType.PrimaryNotEditable),
                            readOnly = true,
                            singleLine = true,
                            label = { Text(stringResource(R.string.forward_entry_proxy)) },
                            trailingIcon = {
                                ExposedDropdownMenuDefaults.TrailingIcon(
                                    expanded = proxyMenuExpanded
                                )
                            }
                        )
                        ExposedDropdownMenu(
                            expanded = proxyMenuExpanded,
                            onDismissRequest = { proxyMenuExpanded = false }
                        ) {
                            proxyOptions.forEach { option ->
                                DropdownMenuItem(
                                    text = {
                                        Text(
                                            option.label.ifEmpty {
                                                stringResource(R.string.forward_entry_proxy_none)
                                            }
                                        )
                                    },
                                    onClick = {
                                        entryProxyId = option.id
                                        proxyMenuExpanded = false
                                    }
                                )
                            }
                        }
                    }
                    JumpChainEditor(
                        hosts = allHosts,
                        targetId = initial?.id,
                        targetName = name,
                        targetAddress = host.takeIf { it.isNotBlank() }?.let { "$username@$it:$port" }.orEmpty(),
                        chain = jumpChain,
                        onChainChange = { jumpChainCsv = it.joinToString(",") },
                        parentListState = editorListState,
                        viewport = editorViewport,
                        enabled = !saving
                    )
                }
            }

            item {
                SectionTitle(title = stringResource(R.string.profile_section_advanced))
            }
            item {
                Column(verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                    Text(
                        text = stringResource(R.string.profile_shell_type),
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    FlowRow(
                        modifier = Modifier
                            .fillMaxWidth()
                            .testTag(ProfileShellTypeTag),
                        horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)
                    ) {
                        ShellType.entries.forEach { option ->
                            FilterChip(
                                selected = shellType == option,
                                onClick = { shellTypeName = option.name },
                                modifier = Modifier.testTag(profileShellOptionTag(option)),
                                label = {
                                    Text(
                                        stringResource(
                                            when (option) {
                                                ShellType.Posix -> R.string.profile_shell_posix
                                                ShellType.Fish -> R.string.profile_shell_fish
                                                ShellType.PowerShell -> R.string.profile_shell_powershell
                                                ShellType.Cmd -> R.string.profile_shell_cmd
                                            }
                                        )
                                    )
                                }
                            )
                        }
                    }
                    Text(
                        text = stringResource(R.string.profile_shell_type_help),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
            item(key = "environment") {
                Column(verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(
                            text = stringResource(R.string.profile_environment_variables),
                            style = MaterialTheme.typography.labelLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.weight(1f)
                        )
                        TextButton(
                            onClick = {
                                environmentFields = ArrayList(environmentFields).apply {
                                    add("")
                                    add("")
                                }
                            },
                            modifier = Modifier.testTag(ProfileEnvironmentAddTag)
                        ) {
                            Icon(Icons.Outlined.Add, contentDescription = null)
                            Text(stringResource(R.string.profile_environment_add))
                        }
                    }
                    environmentVariables.forEachIndexed { index, variable ->
                        val nameInvalid = showValidation && (
                            variable.name.isBlank() && variable.value.isNotEmpty() ||
                                variable.name.isNotBlank() && !ENVIRONMENT_NAME.matches(variable.name.trim())
                            )
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs),
                            verticalAlignment = Alignment.Top
                        ) {
                            OutlinedTextField(
                                value = variable.name,
                                onValueChange = { next ->
                                    environmentFields = ArrayList(environmentFields).apply {
                                        this[index * 2] = next
                                    }
                                },
                                label = { Text(stringResource(R.string.profile_environment_name)) },
                                singleLine = true,
                                isError = nameInvalid,
                                supportingText = if (nameInvalid) {
                                    { Text(stringResource(R.string.profile_environment_name_error)) }
                                } else null,
                                modifier = Modifier.weight(1f)
                                    .testTag(profileEnvironmentNameTag(index)).then(if (index == environmentVariables.indexOfFirst { it.name.trim().isEmpty() && it.value.isNotEmpty() || it.name.isNotBlank() && !ENVIRONMENT_NAME.matches(it.name.trim()) }) Modifier.focusRequester(environmentFocus) else Modifier)
                            )
                            OutlinedTextField(
                                value = variable.value,
                                onValueChange = { next ->
                                    environmentFields = ArrayList(environmentFields).apply {
                                        this[index * 2 + 1] = next
                                    }
                                },
                                label = { Text(stringResource(R.string.profile_environment_value)) },
                                singleLine = true,
                                modifier = Modifier
                                    .weight(1f)
                                    .testTag(profileEnvironmentValueTag(index))
                            )
                            IconButton(
                                onClick = {
                                    environmentFields = ArrayList(environmentFields).apply {
                                        removeAt(index * 2 + 1)
                                        removeAt(index * 2)
                                    }
                                }
                            ) {
                                Icon(
                                    Icons.Outlined.DeleteOutline,
                                    contentDescription = stringResource(
                                        R.string.profile_environment_remove,
                                        variable.name.ifBlank { (index + 1).toString() }
                                    )
                                )
                            }
                        }
                    }
                    if (environmentVariables.isEmpty()) {
                        Text(
                            text = stringResource(R.string.profile_environment_empty),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }
            item {
                OutlinedTextField(
                    value = startupCommand,
                    onValueChange = { startupCommand = it },
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text(stringResource(R.string.profile_startup_command)) }
                )
            }
        }
    }

    if (discardDialogVisible) {
        AlertDialog(
            onDismissRequest = { discardDialogVisible = false },
            title = { Text(stringResource(R.string.profile_discard_title)) },
            text = { Text(stringResource(R.string.profile_discard_message)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        discardDialogVisible = false
                        onDismiss()
                    }
                ) {
                    Text(stringResource(R.string.profile_discard_confirm))
                }
            },
            dismissButton = {
                TextButton(onClick = { discardDialogVisible = false }) {
                    Text(stringResource(R.string.profile_discard_keep))
                }
            }
        )
    }
}
