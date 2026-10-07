package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.ui.uiString

import android.content.Intent
import android.net.Uri
import dev.akko.miaominal.ui.components.WorkspaceBackHandler as BackHandler
import androidx.annotation.StringRes
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.ExperimentalSharedTransitionApi
import androidx.compose.animation.SharedTransitionLayout
import androidx.compose.animation.SharedTransitionScope
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.ArrowForward
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.ArrowDownward
import androidx.compose.material.icons.outlined.Code
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Fingerprint
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Public
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.SwapHoriz
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.Upload
import androidx.compose.material3.Badge
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuAnchorType
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.FloatingActionButton
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.ui.components.PageEmptyState
import dev.akko.miaominal.R
import dev.akko.miaominal.data.BackgroundPolicy
import dev.akko.miaominal.data.ForwardFailureKindModel
import dev.akko.miaominal.data.ForwardKind as StoreForwardKind
import dev.akko.miaominal.data.ForwardPhaseModel
import dev.akko.miaominal.data.ForwardSessionModel
import dev.akko.miaominal.data.ManagedKeyRecord
import dev.akko.miaominal.data.ManagedKeySource as StoreManagedKeySource
import dev.akko.miaominal.data.PortForwardRule
import dev.akko.miaominal.data.ProxyProfile
import dev.akko.miaominal.data.SnippetRecord
import dev.akko.miaominal.ui.LocalMiaominalRepository
import kotlinx.coroutines.launch
import dev.akko.miaominal.ui.components.PageSubtitle
import dev.akko.miaominal.ui.components.SectionTitle
import dev.akko.miaominal.ui.components.MiaominalSearchTopBar
import dev.akko.miaominal.ui.model.HostProfileUi

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ToolDetailScreen(
    destination: ToolDestination,
    profiles: List<HostProfileUi> = emptyList(),
    onDismiss: () -> Unit,
    /** Query carried over from a tools-tab search result. */
    initialSearchQuery: String = "",
    modifier: Modifier = Modifier
) {
    var forwardEditorVisible by rememberSaveable(destination.name) {
        mutableStateOf(false)
    }
    var keychainEditorVisible by rememberSaveable(destination.name) {
        mutableStateOf(false)
    }
    var snippetsEditorVisible by rememberSaveable(destination.name) {
        mutableStateOf(false)
    }
    val navigateBack = {
        if (destination == ToolDestination.Forward && forwardEditorVisible) {
            forwardEditorVisible = false
        } else if (destination == ToolDestination.Keychain && keychainEditorVisible) {
            keychainEditorVisible = false
        } else if (destination == ToolDestination.Snippets && snippetsEditorVisible) {
            snippetsEditorVisible = false
        } else {
            onDismiss()
        }
    }

    BackHandler(
        enabled = (destination == ToolDestination.Forward && forwardEditorVisible) ||
            (destination == ToolDestination.Keychain && keychainEditorVisible) ||
            (destination == ToolDestination.Snippets && snippetsEditorVisible)
    ) {
        when (destination) {
            ToolDestination.Forward -> {
                forwardEditorVisible = false
            }
            ToolDestination.Keychain -> {
                keychainEditorVisible = false
            }
            else -> {
                snippetsEditorVisible = false
            }
        }
    }

    Scaffold(
        modifier = modifier,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(destination.titleRes)) },
                navigationIcon = {
                    IconButton(onClick = navigateBack) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Outlined.ArrowBack,
                            contentDescription = stringResource(R.string.action_back)
                        )
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.surfaceContainer,
                    scrolledContainerColor = MaterialTheme.colorScheme.surfaceContainer
                )
            )
        },
        containerColor = MaterialTheme.colorScheme.surfaceContainer
    ) { contentPadding ->
        val contentModifier = Modifier
            .fillMaxSize()
            .padding(contentPadding)
            .wrapContentWidth().widthIn(max = 720.dp).fillMaxWidth()

        when (destination) {
            ToolDestination.Forward -> ForwardToolPage(
                profiles = profiles,
                editorVisible = forwardEditorVisible,
                onEditorVisibleChange = { forwardEditorVisible = it },
                initialSearchQuery = initialSearchQuery,
                modifier = contentModifier
            )
            ToolDestination.Keychain -> KeychainToolPage(
                profiles = profiles,
                editorVisible = keychainEditorVisible,
                onEditorVisibleChange = { keychainEditorVisible = it },
                initialSearchQuery = initialSearchQuery,
                modifier = contentModifier
            )
            ToolDestination.Snippets -> SnippetsToolPage(
                editorVisible = snippetsEditorVisible,
                onEditorVisibleChange = { snippetsEditorVisible = it },
                initialSearchQuery = initialSearchQuery,
                modifier = contentModifier
            )
            ToolDestination.Trusted -> TrustedHostsToolPage(modifier = contentModifier)
            ToolDestination.Proxies -> ProxyToolPage(modifier = contentModifier)
        }
    }
}

internal enum class ForwardKind(@param:StringRes val labelRes: Int) {
    Local(R.string.forward_local),
    Remote(R.string.forward_remote)
}

/**
 * Browser target of a local forward, mirroring the desktop rule menu: only local
 * rules are reachable from the device browser, and unspecified listeners map to
 * loopback. The stored port wins; a rule configured with port 0 falls back to
 * the port the session actually bound.
 */
internal fun forwardBrowserUrl(
    kind: ForwardKind,
    listenHost: String,
    listenPort: Int,
    boundPort: Int?
): String? {
    if (kind != ForwardKind.Local) return null
    val host = listenHost.trim().let { host ->
        when {
            host.isEmpty() || host == "0.0.0.0" || host == "::" -> "127.0.0.1"
            host.contains(':') -> "[$host]"
            else -> host
        }
    }
    val port = if (listenPort > 0) listenPort else boundPort ?: 0
    if (port <= 0) return null
    return "http://$host:$port"
}

private val ForwardPageEasing = CubicBezierEasing(0.2f, 0f, 0f, 1f)

private data class ForwardRuleUi(
    val id: String,
    val profileId: String,
    val name: String,
    val kind: ForwardKind,
    val sourceHost: String,
    val sourcePort: String,
    val destinationHost: String,
    val destinationPort: String,
    val enabled: Boolean
) {
    // Rule ids are only unique per host ("pf-1", "pf-2", ... per profile), while
    // the repository flattens rules of every profile into one list.
    val key: String get() = "$profileId:$id"
}

private fun ForwardRuleUi.toStoreRule(): PortForwardRule = PortForwardRule(
    id = id,
    profileId = profileId,
    label = name,
    kind = if (kind == ForwardKind.Local) StoreForwardKind.Local else StoreForwardKind.Remote,
    listenHost = sourceHost,
    listenPort = sourcePort.toIntOrNull() ?: 0,
    targetHost = destinationHost,
    targetPort = destinationPort.toIntOrNull() ?: 0,
    enabled = enabled
)

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ForwardToolPage(
    profiles: List<HostProfileUi>,
    editorVisible: Boolean,
    onEditorVisibleChange: (Boolean) -> Unit,
    initialSearchQuery: String,
    modifier: Modifier = Modifier
) {
    val repository = LocalMiaominalRepository.current
    val scope = rememberCoroutineScope()
    val initialProfileId = profiles.firstOrNull()?.id.orEmpty()
    val storedRules by repository.forwards.collectAsState()
    val forwardSessions by repository.forwardSessions.collectAsState()
    val runtimeByKey = remember(forwardSessions) { forwardSessions.associateBy { it.key } }
    val rules = remember(storedRules) {
        storedRules.map { rule ->
            ForwardRuleUi(
                id = rule.id,
                profileId = rule.profileId,
                name = rule.label,
                kind = if (rule.kind == StoreForwardKind.Local) {
                    ForwardKind.Local
                } else {
                    ForwardKind.Remote
                },
                sourceHost = rule.listenHost,
                sourcePort = rule.listenPort.toString(),
                destinationHost = rule.targetHost,
                destinationPort = rule.targetPort.toString(),
                enabled = rule.enabled
            )
        }
    }
    var editingId by rememberSaveable { mutableStateOf<String?>(null) }
    var profileId by rememberSaveable { mutableStateOf(initialProfileId) }
    var name by rememberSaveable { mutableStateOf("") }
    var kindName by rememberSaveable { mutableStateOf(ForwardKind.Local.name) }
    var sourceHost by rememberSaveable { mutableStateOf("127.0.0.1") }
    var sourcePort by rememberSaveable { mutableStateOf("") }
    var destinationHost by rememberSaveable { mutableStateOf("") }
    var destinationPort by rememberSaveable { mutableStateOf("") }
    var validationError by rememberSaveable { mutableStateOf(false) }
    var saveErrorMessage by rememberSaveable { mutableStateOf<String?>(null) }
    var expandedRuleKey by rememberSaveable { mutableStateOf<String?>(null) }
    val kind = ForwardKind.valueOf(kindName)

    fun openNew() {
        editingId = null
        profileId = profiles.firstOrNull()?.id.orEmpty()
        name = ""
        kindName = ForwardKind.Local.name
        sourceHost = "127.0.0.1"
        sourcePort = ""
        destinationHost = ""
        destinationPort = ""
        validationError = false
        saveErrorMessage = null
        onEditorVisibleChange(true)
    }

    fun openEdit(rule: ForwardRuleUi) {
        editingId = rule.id
        profileId = rule.profileId
        name = rule.name
        kindName = rule.kind.name
        sourceHost = rule.sourceHost
        sourcePort = rule.sourcePort
        destinationHost = rule.destinationHost
        destinationPort = rule.destinationPort
        validationError = false
        saveErrorMessage = null
        onEditorVisibleChange(true)
    }

    Box(modifier = modifier) {
        Column(modifier = Modifier.fillMaxSize()) {
            if (!editorVisible) {
                MiaominalSearchTopBar(
                    placeholder = stringResource(R.string.forward_search),
                    modifier = Modifier.fillMaxWidth(),
                    collapsedWindowInsets = WindowInsets(0),
                    initialQuery = initialSearchQuery
                ) { query, collapse ->
                    ForwardSearchResults(
                        query = query,
                        rules = rules,
                        profiles = profiles,
                        runtimeByKey = runtimeByKey,
                        onEnabledChange = { rule, checked ->
                            scope.launch { repository.setForwardEnabled(rule.toStoreRule(), checked) }
                        },
                        onEdit = { rule ->
                            collapse()
                            openEdit(rule)
                        },
                        onDelete = { rule ->
                            scope.launch { repository.deleteForward(rule.toStoreRule()) }
                        }
                    )
                }
            }
            AnimatedContent(
            targetState = editorVisible,
            transitionSpec = {
                if (targetState) {
                    (slideInHorizontally(
                        animationSpec = tween(360, easing = ForwardPageEasing),
                        initialOffsetX = { width -> width }
                    ) + fadeIn(animationSpec = tween(220, delayMillis = 60))) togetherWith
                        (slideOutHorizontally(
                            animationSpec = tween(360, easing = ForwardPageEasing),
                            targetOffsetX = { width -> -width / 4 }
                        ) + fadeOut(animationSpec = tween(180)))
                } else {
                    (slideInHorizontally(
                        animationSpec = tween(360, easing = ForwardPageEasing),
                        initialOffsetX = { width -> -width / 4 }
                    ) + fadeIn(animationSpec = tween(220, delayMillis = 60))) togetherWith
                        (slideOutHorizontally(
                            animationSpec = tween(360, easing = ForwardPageEasing),
                            targetOffsetX = { width -> width }
                        ) + fadeOut(animationSpec = tween(180)))
                }
            },
            modifier = Modifier.weight(1f),
            label = "forward-editor-page"
        ) { showEditor ->
            LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(
                    start = 16.dp,
                    top = 16.dp,
                    end = 16.dp,
                    bottom = if (showEditor) 16.dp else 96.dp
                ),
                verticalArrangement = Arrangement.spacedBy(16.dp)
            ) {
        if (showEditor) {
            item {
                SectionTitle(
                    title = stringResource(
                        if (editingId == null) {
                            R.string.forward_add_rule
                        } else {
                            R.string.forward_edit_rule
                        }
                    )
                )
            }
            item {
                PageSubtitle(text = stringResource(R.string.forward_editor_desc))
            }
            item {
                SectionTitle(title = stringResource(R.string.forward_basic_group))
            }
            item {
                Card(
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
                    )
                ) {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp)
                    ) {
                        ForwardProfileSelector(
                            profiles = profiles,
                            selectedProfileId = profileId,
                            onProfileSelected = { profileId = it }
                        )
                        OutlinedTextField(
                            value = name,
                            onValueChange = { name = it },
                            modifier = Modifier.fillMaxWidth(),
                            singleLine = true,
                            label = { Text(stringResource(R.string.forward_rule_name)) }
                        )
                        Text(
                            stringResource(R.string.forward_type),
                            style = MaterialTheme.typography.labelLarge
                        )
                        SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                            ForwardKind.entries.forEachIndexed { index, option ->
                                SegmentedButton(
                                    selected = kind == option,
                                    onClick = { kindName = option.name },
                                    shape = SegmentedButtonDefaults.itemShape(
                                        index = index,
                                        count = ForwardKind.entries.size
                                    )
                                ) {
                                    Text(stringResource(option.labelRes))
                                }
                            }
                        }
                    }
                }
            }
            item {
                SectionTitle(title = stringResource(R.string.forward_endpoint_group))
            }
            item {
                Card(
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
                    )
                ) {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp)
                    ) {
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            OutlinedTextField(
                                value = sourceHost,
                                onValueChange = { sourceHost = it },
                                modifier = Modifier.weight(2f),
                                singleLine = true,
                                label = { Text(stringResource(R.string.forward_source_host)) }
                            )
                            OutlinedTextField(
                                value = sourcePort,
                                onValueChange = { sourcePort = it.filter(Char::isDigit).take(5) },
                                modifier = Modifier.weight(1f),
                                singleLine = true,
                                label = { Text(stringResource(R.string.forward_source_port)) },
                                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number)
                            )
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            OutlinedTextField(
                                value = destinationHost,
                                onValueChange = { destinationHost = it },
                                modifier = Modifier.weight(2f),
                                singleLine = true,
                                label = {
                                    Text(stringResource(R.string.forward_destination_host))
                                }
                            )
                            OutlinedTextField(
                                value = destinationPort,
                                onValueChange = {
                                    destinationPort = it.filter(Char::isDigit).take(5)
                                },
                                modifier = Modifier.weight(1f),
                                singleLine = true,
                                label = {
                                    Text(stringResource(R.string.forward_destination_port))
                                },
                                keyboardOptions = KeyboardOptions(
                                    keyboardType = KeyboardType.Number
                                )
                            )
                        }
                    }
                }
            }
            item {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    if (validationError) {
                        Text(
                            text = stringResource(R.string.tool_form_required),
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodyMedium
                        )
                    }
                    saveErrorMessage?.let { message ->
                        Text(
                            text = message,
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodyMedium
                        )
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(12.dp)
                    ) {
                        Button(
                            modifier = Modifier.weight(1f),
                            onClick = {
                                val valid = profiles.any { it.id == profileId } &&
                                    name.isNotBlank() && sourcePort.isNotBlank() &&
                                    destinationHost.isNotBlank() &&
                                    destinationPort.isNotBlank()
                                validationError = !valid
                                if (valid) {
                                    val saved = ForwardRuleUi(
                                        id = editingId ?: "forward-${System.currentTimeMillis()}",
                                        profileId = profileId,
                                        name = name.trim(),
                                        kind = kind,
                                        sourceHost = sourceHost.trim(),
                                        sourcePort = sourcePort,
                                        destinationHost = destinationHost.trim(),
                                        destinationPort = destinationPort,
                                        // Runtime start/stop lives on the rule list,
                                        // so editing preserves the stored config flag.
                                        enabled = rules.firstOrNull { it.id == editingId }?.enabled ?: false
                                    )
                                    scope.launch {
                                        val result = repository.saveForward(
                                            PortForwardRule(
                                                id = saved.id,
                                                profileId = saved.profileId,
                                                label = saved.name,
                                                kind = if (saved.kind == ForwardKind.Local) {
                                                    StoreForwardKind.Local
                                                } else {
                                                    StoreForwardKind.Remote
                                                },
                                                listenHost = saved.sourceHost,
                                                listenPort = saved.sourcePort.toIntOrNull() ?: 0,
                                                targetHost = saved.destinationHost,
                                                targetPort = saved.destinationPort.toIntOrNull() ?: 0,
                                                enabled = saved.enabled
                                            )
                                        )
                                        if (result.isSuccess) {
                                            onEditorVisibleChange(false)
                                        } else {
                                            saveErrorMessage = result.exceptionOrNull()
                                                ?.message ?: uiString(R.string.ui_could_not_save)
                                        }
                                    }
                                }
                            }
                        ) {
                            Text(stringResource(R.string.tool_action_save))
                        }
                        OutlinedButton(
                            onClick = { onEditorVisibleChange(false) },
                            modifier = Modifier.weight(1f)
                        ) {
                            Text(stringResource(R.string.tool_action_cancel))
                        }
                    }
                }
            }
        } else {
            item {
                PageSubtitle(text = stringResource(R.string.tool_forward_desc))
            }
            if (rules.isEmpty()) {
                item {
                    PageEmptyState(
                        title = stringResource(R.string.page_empty_title),
                        modifier = Modifier.fillParentMaxHeight(0.65f),
                        icon = Icons.Outlined.SwapHoriz,
                        text = stringResource(R.string.forward_empty)
                    )
                }
            } else {
                items(rules, key = { it.key }) { rule ->
                    val runtime = runtimeByKey[rule.key]
                    ForwardRuleCard(
                        rule = rule,
                        profileName = profiles.firstOrNull { it.id == rule.profileId }?.name
                            ?: stringResource(R.string.forward_profile_missing),
                        runtime = runtime,
                        expanded = expandedRuleKey == rule.key,
                        onExpandedChange = {
                            expandedRuleKey = if (expandedRuleKey == rule.key) null else rule.key
                        },
                        onEnabledChange = { checked ->
                            scope.launch { repository.setForwardEnabled(rule.toStoreRule(), checked) }
                        },
                        onEdit = { openEdit(rule) },
                        onDelete = {
                            if (expandedRuleKey == rule.key) expandedRuleKey = null
                            scope.launch { repository.deleteForward(rule.toStoreRule()) }
                        }
                    )
                }
            }
        }
        }
        }
        }
        AnimatedVisibility(
            visible = !editorVisible,
            modifier = Modifier
                .align(Alignment.BottomEnd)
                .padding(16.dp),
            enter = fadeIn(animationSpec = tween(180, delayMillis = 100)) +
                scaleIn(animationSpec = tween(240, easing = ForwardPageEasing)),
            exit = fadeOut(animationSpec = tween(120)) +
                scaleOut(animationSpec = tween(180, easing = FastOutSlowInEasing))
        ) {
            FloatingActionButton(
                onClick = ::openNew,
                containerColor = MaterialTheme.colorScheme.primaryContainer,
                contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                shape = MaterialTheme.shapes.large
            ) {
                Icon(
                    imageVector = Icons.Outlined.Add,
                    contentDescription = stringResource(R.string.forward_add_rule)
                )
            }
        }
    }
}

@Composable
private fun ForwardSearchResults(
    query: String,
    rules: List<ForwardRuleUi>,
    profiles: List<HostProfileUi>,
    runtimeByKey: Map<String, ForwardSessionModel>,
    onEnabledChange: (ForwardRuleUi, Boolean) -> Unit,
    onEdit: (ForwardRuleUi) -> Unit,
    onDelete: (ForwardRuleUi) -> Unit
) {
    var expandedRuleKey by rememberSaveable { mutableStateOf<String?>(null) }
    val ruleSnapshot = rules.toList()
    val visibleRules = remember(query, ruleSnapshot, profiles) {
        if (query.isEmpty()) {
            ruleSnapshot
        } else {
            ruleSnapshot.filter { rule ->
                val profile = profiles.firstOrNull { it.id == rule.profileId }
                rule.name.contains(query, ignoreCase = true) ||
                    rule.kind.name.contains(query, ignoreCase = true) ||
                    rule.sourceHost.contains(query, ignoreCase = true) ||
                    rule.sourcePort.contains(query, ignoreCase = true) ||
                    rule.destinationHost.contains(query, ignoreCase = true) ||
                    rule.destinationPort.contains(query, ignoreCase = true) ||
                    profile?.name?.contains(query, ignoreCase = true) == true ||
                    profile?.address?.contains(query, ignoreCase = true) == true
            }
        }
    }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(8.dp, 8.dp, 8.dp, 16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        if (visibleRules.isEmpty()) {
            item {
                PageEmptyState(
                        title = stringResource(R.string.page_empty_title),
                        modifier = Modifier.fillParentMaxHeight(0.65f),
                    icon = Icons.Outlined.SwapHoriz,
                    text = stringResource(R.string.forward_search_empty)
                )
            }
        } else {
            items(visibleRules, key = { it.key }) { rule ->
                ForwardRuleCard(
                    rule = rule,
                    profileName = profiles.firstOrNull { it.id == rule.profileId }?.name
                        ?: stringResource(R.string.forward_profile_missing),
                    runtime = runtimeByKey[rule.key],
                    expanded = expandedRuleKey == rule.key,
                    onExpandedChange = {
                        expandedRuleKey = if (expandedRuleKey == rule.key) null else rule.key
                    },
                    onEnabledChange = { checked -> onEnabledChange(rule, checked) },
                    onEdit = { onEdit(rule) },
                    onDelete = { onDelete(rule) }
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ForwardProfileSelector(
    profiles: List<HostProfileUi>,
    selectedProfileId: String,
    onProfileSelected: (String) -> Unit
) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    val selectedProfile = profiles.firstOrNull { it.id == selectedProfileId }
    val selectorEnabled = profiles.isNotEmpty()

    ExposedDropdownMenuBox(
        expanded = expanded,
        onExpandedChange = {
            if (selectorEnabled) expanded = it
        }
    ) {
        OutlinedTextField(
            value = selectedProfile?.name.orEmpty(),
            onValueChange = {},
            modifier = Modifier
                .fillMaxWidth()
                .menuAnchor(
                    ExposedDropdownMenuAnchorType.PrimaryNotEditable,
                    enabled = selectorEnabled
                ),
            enabled = selectorEnabled,
            readOnly = true,
            singleLine = true,
            label = { Text(stringResource(R.string.forward_profile)) },
            placeholder = { Text(stringResource(R.string.forward_profile_placeholder)) },
            supportingText = {
                Text(
                    text = selectedProfile?.let {
                        "${it.username}@${it.address}:${it.port}"
                    } ?: stringResource(R.string.forward_profile_empty)
                )
            },
            leadingIcon = {
                ToolIconTile(Icons.Outlined.Terminal)
            },
            trailingIcon = {
                ExposedDropdownMenuDefaults.TrailingIcon(expanded = expanded)
            }
        )
        ExposedDropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false }
        ) {
            profiles.forEach { profile ->
                DropdownMenuItem(
                    text = {
                        Column {
                            Text(profile.name)
                            Text(
                                text = "${profile.username}@${profile.address}:${profile.port}",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    },
                    leadingIcon = {
                        ToolIconTile(Icons.Outlined.Terminal)
                    },
                    onClick = {
                        onProfileSelected(profile.id)
                        expanded = false
                    }
                )
            }
        }
    }
}

@Composable
private fun ToolIconTile(icon: ImageVector) {
    Surface(
        modifier = Modifier.size(42.dp),
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        shape = CircleShape
    ) {
        Box(contentAlignment = Alignment.Center) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                modifier = Modifier.size(22.dp)
            )
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ForwardRuleCard(
    rule: ForwardRuleUi,
    profileName: String,
    runtime: ForwardSessionModel?,
    expanded: Boolean,
    onExpandedChange: () -> Unit,
    onEnabledChange: (Boolean) -> Unit,
    onEdit: () -> Unit,
    onDelete: () -> Unit
) {
    val source = "${rule.sourceHost}:${rule.sourcePort}"
    val destination = "${rule.destinationHost}:${rule.destinationPort}"
    val chevronRotation by animateFloatAsState(
        targetValue = if (expanded) 180f else 0f,
        animationSpec = tween(durationMillis = 240, easing = FastOutSlowInEasing),
        label = "forward-rule-chevron"
    )
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
        )
    ) {
        Column {
            Row(
                modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 4.dp, bottom = 12.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                FlowRow(
                    modifier = Modifier.weight(1f),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(4.dp)
                ) {
                    Text(
                        text = rule.name,
                        style = MaterialTheme.typography.titleMedium
                    )
                    Badge(
                        modifier = Modifier.defaultMinSize(
                            minWidth = 24.dp,
                            minHeight = 24.dp
                        ),
                        containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                        contentColor = MaterialTheme.colorScheme.onSurfaceVariant
                    ) {
                        Text(
                            text = stringResource(rule.kind.labelRes),
                            modifier = Modifier.padding(horizontal = 4.dp),
                            style = MaterialTheme.typography.labelMedium
                        )
                    }
                    ForwardRuntimeBadge(runtime)
                }
                Spacer(Modifier.width(8.dp))
                // Desktop parity: the switch starts/stops the dedicated session,
                // so it reflects the live runtime state instead of the stored
                // `enabled` flag (which is config-only and never auto-starts).
                val switchLabel = stringResource(R.string.forward_toggle_named, rule.name)
                Switch(
                    checked = runtime?.phase?.isActive == true,
                    onCheckedChange = onEnabledChange,
                    modifier = Modifier.semantics { contentDescription = switchLabel }
                )
                IconButton(onClick = onExpandedChange) {
                    Icon(
                        imageVector = Icons.Filled.KeyboardArrowDown,
                        contentDescription = stringResource(
                            if (expanded) R.string.action_collapse else R.string.action_expand
                        ),
                        modifier = Modifier.graphicsLayer { rotationZ = chevronRotation }
                    )
                }
            }
            AnimatedVisibility(
                visible = expanded,
                enter = expandVertically(
                    animationSpec = tween(durationMillis = 280, easing = FastOutSlowInEasing),
                    expandFrom = Alignment.Top
                ) + fadeIn(animationSpec = tween(durationMillis = 180, delayMillis = 40)),
                exit = shrinkVertically(
                    animationSpec = tween(durationMillis = 240, easing = FastOutSlowInEasing),
                    shrinkTowards = Alignment.Top
                ) + fadeOut(animationSpec = tween(durationMillis = 160))
            ) {
                Column(
                    modifier = Modifier.padding(start = 16.dp, end = 8.dp, bottom = 12.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        ToolIconTile(Icons.Outlined.Terminal)
                        Spacer(Modifier.width(12.dp))
                        Column {
                            Text(
                                text = stringResource(R.string.forward_profile),
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                            Text(
                                text = profileName,
                                style = MaterialTheme.typography.bodyMedium
                            )
                        }
                    }
                    ForwardRoutePanel(
                        source = source,
                        destination = destination,
                        destinationLabel = stringResource(R.string.forward_destination_host)
                    )
                    runtime?.let { ForwardRuntimePanel(runtime = it) }
                    FlowRow(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.End,
                        verticalArrangement = Arrangement.spacedBy(4.dp)
                    ) {
                        val context = LocalContext.current
                        forwardBrowserUrl(
                            kind = rule.kind,
                            listenHost = rule.sourceHost,
                            listenPort = rule.sourcePort.toIntOrNull() ?: 0,
                            boundPort = runtime?.displayPort
                        )?.let { url ->
                            TextButton(
                                onClick = {
                                    val policy = BackgroundPolicy.from(context)
                                    // The browser is another app: without this the
                                    // background grace timer would close the tunnel
                                    // while the user reads the forwarded page.
                                    policy?.beginExternalFlow()
                                    runCatching {
                                        context.startActivity(
                                            Intent(Intent.ACTION_VIEW, Uri.parse(url))
                                        )
                                    }.onFailure { policy?.endExternalFlow() }
                                }
                            ) {
                                Icon(Icons.Outlined.Public, contentDescription = null)
                                Spacer(Modifier.width(8.dp))
                                Text(stringResource(R.string.forward_open_in_browser))
                            }
                        }
                        TextButton(onClick = onEdit) {
                            Icon(Icons.Outlined.Edit, contentDescription = null)
                            Spacer(Modifier.width(8.dp))
                            Text(stringResource(R.string.tool_action_edit))
                        }
                        TextButton(
                            onClick = onDelete,
                            colors = ButtonDefaults.textButtonColors(
                                contentColor = MaterialTheme.colorScheme.error
                            )
                        ) {
                            Icon(Icons.Outlined.Delete, contentDescription = null)
                            Spacer(Modifier.width(8.dp))
                            Text(stringResource(R.string.tool_action_delete))
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ForwardRuntimeBadge(runtime: ForwardSessionModel?) {
    val (label, container, content) = when (runtime?.phase) {
        null -> Triple(
            stringResource(R.string.forward_runtime_idle),
            MaterialTheme.colorScheme.surfaceContainerHighest,
            MaterialTheme.colorScheme.onSurfaceVariant
        )
        ForwardPhaseModel.Starting -> Triple(
            stringResource(R.string.forward_runtime_starting),
            MaterialTheme.colorScheme.tertiaryContainer,
            MaterialTheme.colorScheme.onTertiaryContainer
        )
        ForwardPhaseModel.WaitingForTrust,
        ForwardPhaseModel.WaitingForAuthentication -> Triple(
            stringResource(R.string.forward_runtime_waiting),
            MaterialTheme.colorScheme.tertiaryContainer,
            MaterialTheme.colorScheme.onTertiaryContainer
        )
        ForwardPhaseModel.Listening,
        ForwardPhaseModel.Connected -> Triple(
            stringResource(R.string.forward_runtime_running),
            MaterialTheme.colorScheme.primaryContainer,
            MaterialTheme.colorScheme.onPrimaryContainer
        )
        ForwardPhaseModel.Reconnecting,
        ForwardPhaseModel.Stopping -> Triple(
            stringResource(R.string.forward_runtime_reconnecting),
            MaterialTheme.colorScheme.tertiaryContainer,
            MaterialTheme.colorScheme.onTertiaryContainer
        )
        ForwardPhaseModel.Stopped -> Triple(
            stringResource(R.string.forward_runtime_idle),
            MaterialTheme.colorScheme.surfaceContainerHighest,
            MaterialTheme.colorScheme.onSurfaceVariant
        )
        ForwardPhaseModel.Failed -> Triple(
            stringResource(R.string.forward_runtime_failed),
            MaterialTheme.colorScheme.errorContainer,
            MaterialTheme.colorScheme.onErrorContainer
        )
    }
    Badge(
        modifier = Modifier.defaultMinSize(minWidth = 24.dp, minHeight = 24.dp),
        containerColor = container,
        contentColor = content
    ) {
        Text(
            text = label,
            modifier = Modifier.padding(horizontal = 4.dp),
            style = MaterialTheme.typography.labelMedium
        )
    }
}

@Composable
private fun ForwardRuntimePanel(runtime: ForwardSessionModel) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surfaceContainer,
        shape = MaterialTheme.shapes.medium
    ) {
        Column(
            modifier = Modifier.padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            val statusText = when (runtime.phase) {
                ForwardPhaseModel.Starting -> stringResource(R.string.forward_runtime_starting_desc)
                ForwardPhaseModel.WaitingForTrust ->
                    stringResource(R.string.forward_runtime_trust_desc)
                ForwardPhaseModel.WaitingForAuthentication ->
                    stringResource(R.string.forward_runtime_auth_desc)
                ForwardPhaseModel.Listening, ForwardPhaseModel.Connected ->
                    stringResource(
                        R.string.forward_runtime_listening_desc,
                        runtime.listenHost,
                        runtime.displayPort
                    )
                ForwardPhaseModel.Reconnecting -> stringResource(
                    R.string.forward_runtime_reconnect_desc,
                    runtime.reconnectAttempt,
                    runtime.reconnectError ?: ""
                )
                ForwardPhaseModel.Stopping -> stringResource(R.string.forward_runtime_stopping_desc)
                ForwardPhaseModel.Stopped -> stringResource(R.string.forward_runtime_stopped_desc)
                ForwardPhaseModel.Failed -> runtime.failure?.message
                    ?: stringResource(R.string.forward_runtime_failed_desc)
            }
            Text(
                text = statusText,
                style = MaterialTheme.typography.bodyMedium,
                color = if (runtime.phase == ForwardPhaseModel.Failed) {
                    MaterialTheme.colorScheme.error
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                }
            )
            runtime.failure?.let { failure ->
                ForwardFailureLine(
                    label = stringResource(R.string.forward_failure_node),
                    value = failure.nodeLabel.ifBlank { stringResource(R.string.forward_failure_unknown_node) }
                )
                ForwardFailureLine(
                    label = stringResource(R.string.forward_failure_kind),
                    value = stringResource(failureKindLabel(failure.kind))
                )
            }
            runtime.relayFailure?.let { failure ->
                ForwardFailureLine(
                    label = stringResource(R.string.forward_failure_relay),
                    value = "${failure.nodeLabel} · ${failure.message}"
                )
            }
        }
    }
}

@Composable
private fun ForwardFailureLine(label: String, value: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(
            text = "$label：",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Text(
            text = value,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@StringRes
private fun failureKindLabel(kind: ForwardFailureKindModel): Int = when (kind) {
    ForwardFailureKindModel.PortOccupied -> R.string.forward_failure_port_occupied
    ForwardFailureKindModel.RemoteRefused -> R.string.forward_failure_remote_refused
    ForwardFailureKindModel.TargetUnreachable -> R.string.forward_failure_target_unreachable
    ForwardFailureKindModel.Authentication -> R.string.forward_failure_authentication
    ForwardFailureKindModel.Transport -> R.string.forward_failure_transport
    ForwardFailureKindModel.Other -> R.string.forward_failure_other
}

@Composable
private fun ForwardRoutePanel(
    source: String,
    destination: String,
    destinationLabel: String,
    modifier: Modifier = Modifier
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surfaceContainer,
        shape = MaterialTheme.shapes.medium
    ) {
        BoxWithConstraints {
            if (maxWidth >= 360.dp) {
                Row(
                    modifier = Modifier.padding(12.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    ForwardEndpoint(
                        label = stringResource(R.string.forward_source_host),
                        address = source,
                        modifier = Modifier.weight(1f)
                    )
                    Icon(
                        imageVector = Icons.AutoMirrored.Outlined.ArrowForward,
                        contentDescription = null,
                        modifier = Modifier.padding(horizontal = 12.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    ForwardEndpoint(
                        label = destinationLabel,
                        address = destination,
                        modifier = Modifier.weight(1f)
                    )
                }
            } else {
                Column(
                    modifier = Modifier.padding(12.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    ForwardEndpoint(
                        label = stringResource(R.string.forward_source_host),
                        address = source
                    )
                    Icon(
                        imageVector = Icons.Outlined.ArrowDownward,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    ForwardEndpoint(
                        label = destinationLabel,
                        address = destination
                    )
                }
            }
        }
    }
}

@Composable
private fun ForwardEndpoint(
    label: String,
    address: String,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(2.dp)
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Text(
            text = address,
            style = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis
        )
    }
}

private enum class KeychainView(@param:StringRes val labelRes: Int) {
    ManagedKeys(R.string.keychain_managed_keys),
    AgentIdentities(R.string.keychain_agent_identities)
}

private enum class ManagedKeySource(@param:StringRes val labelRes: Int) {
    Generated(R.string.keychain_source_generated),
    Imported(R.string.keychain_source_imported)
}

private enum class ManagedKeyAlgorithm(@param:StringRes val labelRes: Int, val value: String) {
    Ed25519(R.string.keychain_algorithm_ed25519, "ssh-ed25519"),
    Rsa4096(R.string.keychain_algorithm_rsa4096, "ssh-rsa")
}

private enum class KeychainEditorMode {
    Create,
    Deploy
}

private val DefaultKeychainDeployCommand = """
    if test ! -e ${'$'}1;
    then mkdir -p ${'$'}1;
    chmod 700 ${'$'}1;
    fi;
    if test ! -e "${'$'}1/${'$'}2";
    then touch "${'$'}1/${'$'}2";
    chmod 600 "${'$'}1/${'$'}2";
    fi;
    echo ${'$'}3 >> "${'$'}1/${'$'}2";
""".trimIndent()

private data class ManagedKeyUi(
    val id: String,
    val name: String,
    val algorithm: String,
    val publicKey: String,
    val source: ManagedKeySource
)

private fun ManagedKeyUi.toRecord(): ManagedKeyRecord = ManagedKeyRecord(
    id = id,
    name = name,
    algorithm = algorithm,
    publicKey = publicKey,
    source = if (source == ManagedKeySource.Generated) {
        StoreManagedKeySource.Generated
    } else {
        StoreManagedKeySource.Imported
    }
)

private data class AgentIdentityUi(
    val id: String,
    val label: String,
    val comment: String,
    val kind: String,
    val serialized: String
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun KeychainToolPage(
    profiles: List<HostProfileUi>,
    editorVisible: Boolean,
    onEditorVisibleChange: (Boolean) -> Unit,
    initialSearchQuery: String,
    modifier: Modifier = Modifier
) {
    val repository = LocalMiaominalRepository.current
    val scope = rememberCoroutineScope()
    val storedManagedKeys by repository.managedKeys.collectAsState()
    val managedKeys = remember(storedManagedKeys) {
        storedManagedKeys.map { key ->
            ManagedKeyUi(
                id = key.id,
                name = key.name,
                algorithm = key.algorithm,
                publicKey = key.publicKey,
                source = if (key.source == StoreManagedKeySource.Generated) {
                    ManagedKeySource.Generated
                } else {
                    ManagedKeySource.Imported
                }
            )
        }
    }
    val agentIdentities = remember { emptyList<AgentIdentityUi>() }
    var viewName by rememberSaveable { mutableStateOf(KeychainView.ManagedKeys.name) }
    var expandedItemId by rememberSaveable { mutableStateOf<String?>(null) }
    var editorModeName by rememberSaveable { mutableStateOf(KeychainEditorMode.Create.name) }
    var selectedKeyId by rememberSaveable { mutableStateOf<String?>(null) }
    var renameKeyId by rememberSaveable { mutableStateOf<String?>(null) }
    var renameValue by rememberSaveable { mutableStateOf("") }
    var renameValidationError by rememberSaveable { mutableStateOf(false) }
    var deleteKeyId by rememberSaveable { mutableStateOf<String?>(null) }
    var keyName by rememberSaveable { mutableStateOf("") }
    var sourceName by rememberSaveable { mutableStateOf(ManagedKeySource.Generated.name) }
    var algorithmName by rememberSaveable { mutableStateOf(ManagedKeyAlgorithm.Ed25519.name) }
    var privateKeyMaterial by rememberSaveable { mutableStateOf("") }
    var publicKeyMaterial by rememberSaveable { mutableStateOf("") }
    var passphrase by rememberSaveable { mutableStateOf("") }
    var selectedProfileId by rememberSaveable { mutableStateOf("") }
    var deployLocation by rememberSaveable { mutableStateOf(".ssh") }
    var deployFilename by rememberSaveable { mutableStateOf("authorized_keys") }
    var deployCommand by rememberSaveable { mutableStateOf(DefaultKeychainDeployCommand) }
    var validationError by rememberSaveable { mutableStateOf(false) }
    var saveErrorMessage by rememberSaveable { mutableStateOf<String?>(null) }
    var statusMessage by rememberSaveable { mutableStateOf<String?>(null) }
    val selectedView = KeychainView.valueOf(viewName)
    val editorMode = KeychainEditorMode.valueOf(editorModeName)
    val selectedSource = ManagedKeySource.valueOf(sourceName)
    val selectedAlgorithm = ManagedKeyAlgorithm.valueOf(algorithmName)
    val selectedKey = managedKeys.firstOrNull { it.id == selectedKeyId }
    val renameKey = managedKeys.firstOrNull { it.id == renameKeyId }
    val deleteKey = managedKeys.firstOrNull { it.id == deleteKeyId }
    val publicKeyPlaceholder = stringResource(R.string.keychain_public_key_placeholder)
    val generatedStatusMessage = stringResource(
        R.string.keychain_generated_message,
        keyName.trim()
    )
    val importedStatusMessage = stringResource(
        R.string.keychain_imported_message,
        keyName.trim()
    )
    val selectedProfileName = profiles.firstOrNull { it.id == selectedProfileId }?.name.orEmpty()
    val deployedStatusMessage = stringResource(
        R.string.keychain_deployed_message,
        selectedKey?.name.orEmpty(),
        selectedProfileName
    )
    val refreshedStatusMessage = stringResource(R.string.keychain_refreshed_message)

    fun openCreate() {
        editorModeName = KeychainEditorMode.Create.name
        selectedKeyId = null
        keyName = ""
        sourceName = ManagedKeySource.Generated.name
        algorithmName = ManagedKeyAlgorithm.Ed25519.name
        privateKeyMaterial = ""
        publicKeyMaterial = ""
        passphrase = ""
        validationError = false
        onEditorVisibleChange(true)
    }

    fun openRename(key: ManagedKeyUi) {
        renameKeyId = key.id
        renameValue = key.name
        renameValidationError = false
    }

    fun openDeploy(key: ManagedKeyUi) {
        editorModeName = KeychainEditorMode.Deploy.name
        selectedKeyId = key.id
        selectedProfileId = profiles.firstOrNull()?.id.orEmpty()
        deployLocation = ".ssh"
        deployFilename = "authorized_keys"
        deployCommand = DefaultKeychainDeployCommand
        validationError = false
        onEditorVisibleChange(true)
    }

    Box(modifier = modifier) {
        Column(modifier = Modifier.fillMaxSize()) {
            if (!editorVisible) {
                MiaominalSearchTopBar(
                    placeholder = stringResource(R.string.keychain_search),
                    modifier = Modifier.fillMaxWidth(),
                    collapsedWindowInsets = WindowInsets(0),
                    initialQuery = initialSearchQuery,
                    collapsedTrailingContent = {
                        IconButton(onClick = {
                            statusMessage = refreshedStatusMessage
                        }) {
                            Icon(
                                imageVector = Icons.Outlined.Refresh,
                                contentDescription = stringResource(R.string.keychain_refresh)
                            )
                        }
                    }
                ) { searchQuery, collapse ->
                    KeychainSearchResults(
                        query = searchQuery,
                        selectedView = selectedView,
                        managedKeys = managedKeys,
                        agentIdentities = agentIdentities,
                        onDeploy = { key ->
                            collapse()
                            openDeploy(key)
                        },
                        onRename = { key ->
                            collapse()
                            openRename(key)
                        },
                        onDelete = { key ->
                            collapse()
                            deleteKeyId = key.id
                        }
                    )
                }
            }
            AnimatedContent(
            targetState = editorVisible,
            modifier = Modifier.weight(1f),
            transitionSpec = {
                if (targetState) {
                    (slideInHorizontally(
                        animationSpec = tween(320, easing = ForwardPageEasing),
                        initialOffsetX = { it / 4 }
                    ) + fadeIn(tween(220))) togetherWith
                        (slideOutHorizontally(
                            animationSpec = tween(280, easing = ForwardPageEasing),
                            targetOffsetX = { -it / 7 }
                        ) + fadeOut(tween(180)))
                } else {
                    (slideInHorizontally(
                        animationSpec = tween(320, easing = ForwardPageEasing),
                        initialOffsetX = { -it / 7 }
                    ) + fadeIn(tween(220))) togetherWith
                        (slideOutHorizontally(
                            animationSpec = tween(280, easing = ForwardPageEasing),
                            targetOffsetX = { it / 4 }
                        ) + fadeOut(tween(180)))
                }
            },
            label = "keychain-page"
        ) { showEditor ->
            if (showEditor) {
                LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(16.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp)
                ) {
                    item {
                        SectionTitle(
                            title = stringResource(
                                when (editorMode) {
                                    KeychainEditorMode.Create -> R.string.keychain_editor_title
                                    KeychainEditorMode.Deploy -> R.string.keychain_deploy_title
                                }
                            )
                        )
                    }
                    item {
                        Card(
                            colors = CardDefaults.cardColors(
                                containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
                            )
                        ) {
                            Column(
                                modifier = Modifier.padding(16.dp),
                                verticalArrangement = Arrangement.spacedBy(16.dp)
                            ) {
                                when (editorMode) {
                                    KeychainEditorMode.Create -> {
                                        OutlinedTextField(
                                            value = keyName,
                                            onValueChange = { keyName = it },
                                            modifier = Modifier.fillMaxWidth(),
                                            singleLine = true,
                                            label = { Text(stringResource(R.string.keychain_key_name)) }
                                        )
                                        Text(
                                            stringResource(R.string.keychain_source),
                                            style = MaterialTheme.typography.labelLarge
                                        )
                                        SingleChoiceSegmentedButtonRow(
                                            modifier = Modifier.fillMaxWidth()
                                        ) {
                                            ManagedKeySource.entries.forEachIndexed { index, source ->
                                                SegmentedButton(
                                                    selected = selectedSource == source,
                                                    onClick = { sourceName = source.name },
                                                    shape = SegmentedButtonDefaults.itemShape(
                                                        index = index,
                                                        count = ManagedKeySource.entries.size
                                                    ),
                                                    label = { Text(stringResource(source.labelRes)) }
                                                )
                                            }
                                        }
                                        if (selectedSource == ManagedKeySource.Generated) {
                                            Text(
                                                stringResource(R.string.keychain_algorithm),
                                                style = MaterialTheme.typography.labelLarge
                                            )
                                            SingleChoiceSegmentedButtonRow(
                                                modifier = Modifier.fillMaxWidth()
                                            ) {
                                                ManagedKeyAlgorithm.entries.forEachIndexed { index, algorithm ->
                                                    SegmentedButton(
                                                        selected = selectedAlgorithm == algorithm,
                                                        onClick = { algorithmName = algorithm.name },
                                                        shape = SegmentedButtonDefaults.itemShape(
                                                            index = index,
                                                            count = ManagedKeyAlgorithm.entries.size
                                                        ),
                                                        label = { Text(stringResource(algorithm.labelRes)) }
                                                    )
                                                }
                                            }
                                            Text(
                                                text = stringResource(R.string.keychain_generate_desc),
                                                style = MaterialTheme.typography.bodyMedium,
                                                color = MaterialTheme.colorScheme.onSurfaceVariant
                                            )
                                        } else {
                                            OutlinedTextField(
                                                value = privateKeyMaterial,
                                                onValueChange = { privateKeyMaterial = it },
                                                modifier = Modifier.fillMaxWidth(),
                                                minLines = 5,
                                                label = { Text(stringResource(R.string.keychain_private_key)) },
                                                textStyle = MaterialTheme.typography.bodyMedium.copy(
                                                    fontFamily = FontFamily.Monospace
                                                )
                                            )
                                            OutlinedTextField(
                                                value = publicKeyMaterial,
                                                onValueChange = { publicKeyMaterial = it },
                                                modifier = Modifier.fillMaxWidth(),
                                                minLines = 2,
                                                label = { Text(stringResource(R.string.keychain_public_key_optional)) },
                                                textStyle = MaterialTheme.typography.bodyMedium.copy(
                                                    fontFamily = FontFamily.Monospace
                                                )
                                            )
                                            OutlinedTextField(
                                                value = passphrase,
                                                onValueChange = { passphrase = it },
                                                modifier = Modifier.fillMaxWidth(),
                                                singleLine = true,
                                                label = { Text(stringResource(R.string.keychain_passphrase_optional)) },
                                                visualTransformation = PasswordVisualTransformation()
                                            )
                                        }
                                    }

                                    KeychainEditorMode.Deploy -> {
                                        selectedKey?.let { key ->
                                            ManagedKeySummary(key)
                                        }
                                        KeychainProfileSelector(
                                            profiles = profiles,
                                            selectedProfileId = selectedProfileId,
                                            onProfileSelected = { selectedProfileId = it }
                                        )
                                        OutlinedTextField(
                                            value = deployLocation,
                                            onValueChange = { deployLocation = it },
                                            modifier = Modifier.fillMaxWidth(),
                                            singleLine = true,
                                            label = { Text(stringResource(R.string.keychain_deploy_location)) }
                                        )
                                        OutlinedTextField(
                                            value = deployFilename,
                                            onValueChange = { deployFilename = it },
                                            modifier = Modifier.fillMaxWidth(),
                                            singleLine = true,
                                            label = { Text(stringResource(R.string.keychain_deploy_filename)) }
                                        )
                                        OutlinedTextField(
                                            value = deployCommand,
                                            onValueChange = { deployCommand = it },
                                            modifier = Modifier.fillMaxWidth(),
                                            minLines = 8,
                                            label = { Text(stringResource(R.string.keychain_deploy_command)) },
                                            supportingText = {
                                                Text(stringResource(R.string.keychain_deploy_command_desc))
                                            },
                                            textStyle = MaterialTheme.typography.bodyMedium.copy(
                                                fontFamily = FontFamily.Monospace
                                            )
                                        )
                                    }
                                }
                                if (validationError) {
                                    Text(
                                        text = stringResource(R.string.tool_form_required),
                                        color = MaterialTheme.colorScheme.error,
                                        style = MaterialTheme.typography.bodyMedium
                                    )
                                }
                                saveErrorMessage?.let { message ->
                                    Text(
                                        text = message,
                                        color = MaterialTheme.colorScheme.error,
                                        style = MaterialTheme.typography.bodyMedium
                                    )
                                }
                                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    Button(
                                        onClick = {
                                            when (editorMode) {
                                                KeychainEditorMode.Create -> {
                                                    val valid = keyName.isNotBlank() &&
                                                        (selectedSource == ManagedKeySource.Generated ||
                                                            privateKeyMaterial.isNotBlank())
                                                    validationError = !valid
                                                    if (valid) {
                                                        scope.launch {
                                                            val result = if (
                                                                selectedSource == ManagedKeySource.Generated
                                                            ) {
                                                                repository.generateKey(
                                                                    keyName.trim(),
                                                                    selectedAlgorithm.value
                                                                )
                                                            } else {
                                                                repository.importKey(
                                                                    keyName.trim(),
                                                                    privateKeyMaterial,
                                                                    passphrase.ifBlank { null }
                                                                )
                                                            }
                                                            if (result.isSuccess) {
                                                                statusMessage = if (
                                                                    selectedSource == ManagedKeySource.Generated
                                                                ) {
                                                                    generatedStatusMessage
                                                                } else {
                                                                    importedStatusMessage
                                                                }
                                                                onEditorVisibleChange(false)
                                                            } else {
                                                                saveErrorMessage = result.exceptionOrNull()
                                                                    ?.message ?: uiString(R.string.ui_could_not_save)
                                                            }
                                                        }
                                                    }
                                                }

                                                KeychainEditorMode.Deploy -> {
                                                    val valid = selectedKey != null &&
                                                        selectedProfileId.isNotBlank() &&
                                                        deployLocation.isNotBlank() &&
                                                        deployFilename.isNotBlank()
                                                    validationError = !valid
                                                    if (valid) {
                                                        statusMessage = deployedStatusMessage
                                                        onEditorVisibleChange(false)
                                                    }
                                                }
                                            }
                                        }
                                    ) {
                                        Text(
                                            stringResource(
                                                when (editorMode) {
                                                    KeychainEditorMode.Create -> {
                                                        if (selectedSource == ManagedKeySource.Generated) {
                                                            R.string.keychain_generate
                                                        } else {
                                                            R.string.keychain_import
                                                        }
                                                    }
                                                    KeychainEditorMode.Deploy -> R.string.keychain_deploy
                                                }
                                            )
                                        )
                                    }
                                    TextButton(onClick = { onEditorVisibleChange(false) }) {
                                        Text(stringResource(R.string.tool_action_cancel))
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(
                        start = 16.dp,
                        top = 16.dp,
                        end = 16.dp,
                        bottom = if (selectedView == KeychainView.ManagedKeys) 96.dp else 16.dp
                    ),
                    verticalArrangement = Arrangement.spacedBy(16.dp)
                ) {
                    item { PageSubtitle(text = stringResource(R.string.tool_keychain_desc)) }
                    item {
                        SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                            KeychainView.entries.forEachIndexed { index, view ->
                                SegmentedButton(
                                    selected = selectedView == view,
                                    onClick = {
                                        viewName = view.name
                                        expandedItemId = null
                                    },
                                    shape = SegmentedButtonDefaults.itemShape(
                                        index = index,
                                        count = KeychainView.entries.size
                                    ),
                                    label = { Text(stringResource(view.labelRes)) }
                                )
                            }
                        }
                    }
                    statusMessage?.let { message ->
                        item {
                            StatusCard(
                                icon = Icons.Outlined.Key,
                                title = stringResource(R.string.tool_keychain),
                                body = message
                            )
                        }
                    }
                    if ((selectedView == KeychainView.ManagedKeys && managedKeys.isNotEmpty()) ||
                        (selectedView == KeychainView.AgentIdentities && agentIdentities.isNotEmpty())) item {
                        SectionTitle(title = stringResource(selectedView.labelRes))
                    }
                    when (selectedView) {
                        KeychainView.ManagedKeys -> {
                            if (managedKeys.isEmpty()) {
                                item {
                                    PageEmptyState(
                                        icon = Icons.Outlined.Key,
                                        title = stringResource(R.string.keychain_managed_empty_title),
                                        text = stringResource(R.string.keychain_managed_empty),
                                        modifier = Modifier.fillParentMaxHeight(0.6f)
                                    )
                                }
                            } else {
                                items(managedKeys, key = { it.id }) { key ->
                                    ManagedKeyCard(
                                        key = key,
                                        expanded = expandedItemId == key.id,
                                        onExpandedChange = {
                                            expandedItemId = if (expandedItemId == key.id) {
                                                null
                                            } else {
                                                key.id
                                            }
                                        },
                                        onDeploy = { openDeploy(key) },
                                        onRename = { openRename(key) },
                                        onDelete = {
                                            deleteKeyId = key.id
                                        }
                                    )
                                }
                            }
                        }

                        KeychainView.AgentIdentities -> {
                            if (agentIdentities.isEmpty()) {
                                item {
                                    PageEmptyState(
                                        icon = Icons.Outlined.Fingerprint,
                                        title = stringResource(R.string.keychain_agent_empty_title),
                                        text = stringResource(R.string.keychain_agent_empty),
                                        modifier = Modifier.fillParentMaxHeight(0.6f)
                                    )
                                }
                            } else {
                                items(agentIdentities, key = { it.id }) { identity ->
                                    AgentIdentityCard(
                                        identity = identity,
                                        expanded = expandedItemId == identity.id,
                                        onExpandedChange = {
                                            expandedItemId = if (expandedItemId == identity.id) {
                                                null
                                            } else {
                                                identity.id
                                            }
                                        }
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
        }
        AnimatedVisibility(
            visible = !editorVisible && selectedView == KeychainView.ManagedKeys,
            modifier = Modifier
                .align(Alignment.BottomEnd)
                .padding(16.dp),
            enter = fadeIn(animationSpec = tween(180, delayMillis = 100)) +
                scaleIn(animationSpec = tween(240, easing = ForwardPageEasing)),
            exit = fadeOut(animationSpec = tween(120)) +
                scaleOut(animationSpec = tween(180, easing = FastOutSlowInEasing))
        ) {
            FloatingActionButton(
                onClick = ::openCreate,
                containerColor = MaterialTheme.colorScheme.primaryContainer,
                contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                shape = MaterialTheme.shapes.large
            ) {
                Icon(
                    imageVector = Icons.Outlined.Add,
                    contentDescription = stringResource(R.string.keychain_add_key)
                )
            }
        }
        renameKey?.let { key ->
            AlertDialog(
                onDismissRequest = { renameKeyId = null },
                title = { Text(stringResource(R.string.keychain_rename_title)) },
                text = {
                    OutlinedTextField(
                        value = renameValue,
                        onValueChange = {
                            renameValue = it
                            renameValidationError = false
                        },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        isError = renameValidationError,
                        label = { Text(stringResource(R.string.keychain_key_name)) },
                        supportingText = if (renameValidationError) {
                            { Text(stringResource(R.string.tool_form_required)) }
                        } else {
                            null
                        }
                    )
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            renameValidationError = renameValue.isBlank()
                            if (!renameValidationError) {
                                scope.launch {
                                    val result = repository.saveKey(
                                        key.copy(name = renameValue.trim()).toRecord()
                                    )
                                    if (result.isSuccess) {
                                        renameKeyId = null
                                    } else {
                                        saveErrorMessage = result.exceptionOrNull()
                                            ?.message ?: uiString(R.string.ui_could_not_save)
                                    }
                                }
                            }
                        }
                    ) {
                        Text(stringResource(R.string.keychain_rename))
                    }
                },
                dismissButton = {
                    TextButton(onClick = { renameKeyId = null }) {
                        Text(stringResource(R.string.tool_action_cancel))
                    }
                }
            )
        }
        deleteKey?.let { key ->
            AlertDialog(
                onDismissRequest = { deleteKeyId = null },
                icon = { Icon(Icons.Outlined.Delete, contentDescription = null) },
                title = { Text(stringResource(R.string.keychain_delete_title)) },
                text = { Text(stringResource(R.string.keychain_delete_message, key.name)) },
                confirmButton = {
                    TextButton(
                        onClick = {
                            if (expandedItemId == key.id) expandedItemId = null
                            scope.launch { repository.deleteKey(key.id) }
                            deleteKeyId = null
                        },
                        colors = ButtonDefaults.textButtonColors(
                            contentColor = MaterialTheme.colorScheme.error                        )
                    ) {
                        Text(stringResource(R.string.tool_action_delete))
                    }
                },
                dismissButton = {
                    TextButton(onClick = { deleteKeyId = null }) {
                        Text(stringResource(R.string.tool_action_cancel))
                    }
                }
            )
        }
    }
}

@Composable
private fun KeychainSearchResults(
    query: String,
    selectedView: KeychainView,
    managedKeys: List<ManagedKeyUi>,
    agentIdentities: List<AgentIdentityUi>,
    onDeploy: (ManagedKeyUi) -> Unit,
    onRename: (ManagedKeyUi) -> Unit,
    onDelete: (ManagedKeyUi) -> Unit
) {
    var expandedItemId by rememberSaveable(selectedView.name) { mutableStateOf<String?>(null) }
    val managedKeySnapshot = managedKeys.toList()
    val agentIdentitySnapshot = agentIdentities.toList()
    val visibleManagedKeys = remember(query, managedKeySnapshot) {
        if (query.isEmpty()) {
            managedKeySnapshot
        } else {
            managedKeySnapshot.filter { key ->
                key.name.contains(query, ignoreCase = true) ||
                    key.algorithm.contains(query, ignoreCase = true) ||
                    key.publicKey.contains(query, ignoreCase = true) ||
                    key.source.name.contains(query, ignoreCase = true)
            }
        }
    }
    val visibleAgentIdentities = remember(query, agentIdentitySnapshot) {
        if (query.isEmpty()) {
            agentIdentitySnapshot
        } else {
            agentIdentitySnapshot.filter { identity ->
                identity.label.contains(query, ignoreCase = true) ||
                    identity.comment.contains(query, ignoreCase = true) ||
                    identity.kind.contains(query, ignoreCase = true) ||
                    identity.serialized.contains(query, ignoreCase = true)
            }
        }
    }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(8.dp, 8.dp, 8.dp, 16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        when (selectedView) {
            KeychainView.ManagedKeys -> {
                if (visibleManagedKeys.isEmpty()) {
                    item {
                        PageEmptyState(
                            icon = Icons.Outlined.Search,
                            title = stringResource(R.string.keychain_no_match),
                            text = stringResource(R.string.keychain_search_empty_hint),
                            modifier = Modifier.fillParentMaxHeight()
                        )
                    }
                } else {
                    items(visibleManagedKeys, key = { it.id }) { key ->
                        ManagedKeyCard(
                            key = key,
                            expanded = expandedItemId == key.id,
                            onExpandedChange = {
                                expandedItemId = if (expandedItemId == key.id) null else key.id
                            },
                            onDeploy = { onDeploy(key) },
                            onRename = { onRename(key) },
                            onDelete = { onDelete(key) }
                        )
                    }
                }
            }

            KeychainView.AgentIdentities -> {
                if (visibleAgentIdentities.isEmpty()) {
                    item {
                        PageEmptyState(
                            icon = Icons.Outlined.Search,
                            title = stringResource(R.string.keychain_no_match),
                            text = stringResource(R.string.keychain_search_empty_hint),
                            modifier = Modifier.fillParentMaxHeight()
                        )
                    }
                } else {
                    items(visibleAgentIdentities, key = { it.id }) { identity ->
                        AgentIdentityCard(
                            identity = identity,
                            expanded = expandedItemId == identity.id,
                            onExpandedChange = {
                                expandedItemId = if (expandedItemId == identity.id) {
                                    null
                                } else {
                                    identity.id
                                }
                            }
                        )
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ManagedKeySummary(key: ManagedKeyUi) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surfaceContainer,
        shape = MaterialTheme.shapes.medium
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Text(key.name, style = MaterialTheme.typography.titleMedium)
            Text(
                key.algorithm,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp)
            ) {
                KeychainBadge(stringResource(key.source.labelRes))
                KeychainBadge(stringResource(R.string.keychain_os_keyring))
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ManagedKeyCard(
    key: ManagedKeyUi,
    expanded: Boolean,
    onExpandedChange: () -> Unit,
    onDeploy: () -> Unit,
    onRename: () -> Unit,
    onDelete: () -> Unit
) {
    val chevronRotation by animateFloatAsState(
        targetValue = if (expanded) 180f else 0f,
        animationSpec = tween(durationMillis = 240, easing = FastOutSlowInEasing),
        label = "managed-key-chevron"
    )
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
        )
    ) {
        Column {
            Row(
                modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 4.dp, bottom = 12.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                ToolIconTile(Icons.Outlined.Key)
                Spacer(Modifier.width(12.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = key.name,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                    Text(
                        text = key.algorithm,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                IconButton(onClick = onExpandedChange) {
                    Icon(
                        imageVector = Icons.Filled.KeyboardArrowDown,
                        contentDescription = stringResource(
                            if (expanded) R.string.action_collapse else R.string.action_expand
                        ),
                        modifier = Modifier.graphicsLayer { rotationZ = chevronRotation }
                    )
                }
            }
            AnimatedVisibility(
                visible = expanded,
                enter = expandVertically(
                    animationSpec = tween(280, easing = FastOutSlowInEasing),
                    expandFrom = Alignment.Top
                ) + fadeIn(tween(180, delayMillis = 40)),
                exit = shrinkVertically(
                    animationSpec = tween(240, easing = FastOutSlowInEasing),
                    shrinkTowards = Alignment.Top
                ) + fadeOut(tween(160))
            ) {
                Column(
                    modifier = Modifier.padding(start = 16.dp, end = 8.dp, bottom = 12.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp)
                    ) {
                        KeychainBadge(stringResource(key.source.labelRes))
                        KeychainBadge(stringResource(R.string.keychain_os_keyring))
                    }
                    Surface(
                        modifier = Modifier.fillMaxWidth(),
                        color = MaterialTheme.colorScheme.surfaceContainer,
                        shape = MaterialTheme.shapes.medium
                    ) {
                        Column(modifier = Modifier.padding(12.dp)) {
                            Text(
                                text = stringResource(R.string.keychain_public_key),
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                            Text(
                                text = key.publicKey,
                                style = MaterialTheme.typography.bodyMedium.copy(
                                    fontFamily = FontFamily.Monospace
                                ),
                                maxLines = 3,
                                overflow = TextOverflow.Ellipsis
                            )
                        }
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.End
                    ) {
                        TextButton(onClick = onDeploy) {
                            Icon(Icons.Outlined.Upload, contentDescription = null)
                            Spacer(Modifier.width(8.dp))
                            Text(stringResource(R.string.keychain_deploy))
                        }
                        TextButton(onClick = onRename) {
                            Icon(Icons.Outlined.Edit, contentDescription = null)
                            Spacer(Modifier.width(8.dp))
                            Text(stringResource(R.string.keychain_rename))
                        }
                        TextButton(
                            onClick = onDelete,
                            colors = ButtonDefaults.textButtonColors(
                                contentColor = MaterialTheme.colorScheme.error
                            )
                        ) {
                            Icon(Icons.Outlined.Delete, contentDescription = null)
                            Spacer(Modifier.width(8.dp))
                            Text(stringResource(R.string.tool_action_delete))
                        }
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AgentIdentityCard(
    identity: AgentIdentityUi,
    expanded: Boolean,
    onExpandedChange: () -> Unit
) {
    val chevronRotation by animateFloatAsState(
        targetValue = if (expanded) 180f else 0f,
        animationSpec = tween(durationMillis = 240, easing = FastOutSlowInEasing),
        label = "agent-identity-chevron"
    )
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
        )
    ) {
        Column {
            Row(
                modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 4.dp, bottom = 12.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                ToolIconTile(Icons.Outlined.Fingerprint)
                Spacer(Modifier.width(12.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = identity.label,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                    Text(
                        text = identity.kind,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                IconButton(onClick = onExpandedChange) {
                    Icon(
                        imageVector = Icons.Filled.KeyboardArrowDown,
                        contentDescription = stringResource(
                            if (expanded) R.string.action_collapse else R.string.action_expand
                        ),
                        modifier = Modifier.graphicsLayer { rotationZ = chevronRotation }
                    )
                }
            }
            AnimatedVisibility(
                visible = expanded,
                enter = expandVertically(
                    animationSpec = tween(280, easing = FastOutSlowInEasing),
                    expandFrom = Alignment.Top
                ) + fadeIn(tween(180, delayMillis = 40)),
                exit = shrinkVertically(
                    animationSpec = tween(240, easing = FastOutSlowInEasing),
                    shrinkTowards = Alignment.Top
                ) + fadeOut(tween(160))
            ) {
                Column(
                    modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    Text(
                        text = identity.comment.ifBlank {
                            stringResource(R.string.keychain_no_comment)
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp)
                    ) {
                        KeychainBadge(identity.kind)
                        KeychainBadge(stringResource(R.string.keychain_agent_available))
                    }
                }
            }
        }
    }
}

@Composable
private fun KeychainBadge(text: String) {
    Badge(
        modifier = Modifier.defaultMinSize(minWidth = 24.dp, minHeight = 24.dp),
        containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
        contentColor = MaterialTheme.colorScheme.onSurfaceVariant
    ) {
        Text(
            text = text,
            modifier = Modifier.padding(horizontal = 4.dp),
            style = MaterialTheme.typography.labelMedium
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun KeychainProfileSelector(
    profiles: List<HostProfileUi>,
    selectedProfileId: String,
    onProfileSelected: (String) -> Unit
) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    val selectedProfile = profiles.firstOrNull { it.id == selectedProfileId }

    ExposedDropdownMenuBox(
        expanded = expanded,
        onExpandedChange = { if (profiles.isNotEmpty()) expanded = it }
    ) {
        OutlinedTextField(
            value = selectedProfile?.name.orEmpty(),
            onValueChange = {},
            modifier = Modifier
                .fillMaxWidth()
                .menuAnchor(
                    ExposedDropdownMenuAnchorType.PrimaryNotEditable,
                    enabled = profiles.isNotEmpty()
                ),
            enabled = profiles.isNotEmpty(),
            readOnly = true,
            singleLine = true,
            label = { Text(stringResource(R.string.keychain_deploy_profile)) },
            placeholder = { Text(stringResource(R.string.keychain_select_profile)) },
            supportingText = {
                Text(
                    selectedProfile?.let {
                        "${it.username}@${it.address}:${it.port}"
                    } ?: stringResource(R.string.keychain_no_profiles)
                )
            },
            leadingIcon = { ToolIconTile(Icons.Outlined.Terminal) },
            trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded = expanded) }
        )
        ExposedDropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false }
        ) {
            profiles.forEach { profile ->
                DropdownMenuItem(
                    text = {
                        Column {
                            Text(profile.name)
                            Text(
                                text = "${profile.username}@${profile.address}:${profile.port}",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    },
                    leadingIcon = { ToolIconTile(Icons.Outlined.Terminal) },
                    onClick = {
                        onProfileSelected(profile.id)
                        expanded = false
                    }
                )
            }
        }
    }
}

private data class SnippetUi(
    val id: String,
    val description: String,
    val packageName: String,
    val language: String,
    val script: String
)

@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
private fun SnippetsToolPage(
    editorVisible: Boolean,
    onEditorVisibleChange: (Boolean) -> Unit,
    initialSearchQuery: String,
    modifier: Modifier = Modifier
) {
    val repository = LocalMiaominalRepository.current
    val scope = rememberCoroutineScope()
    val clipboardManager = LocalClipboardManager.current
    val storedSnippets by repository.snippets.collectAsState()
    val snippets = remember(storedSnippets) {
        storedSnippets.map { snippet ->
            SnippetUi(
                id = snippet.id,
                description = snippet.description,
                packageName = snippet.packageName,
                language = snippet.language,
                script = snippet.script
            )
        }
    }
    var editingId by rememberSaveable { mutableStateOf<String?>(null) }
    var description by rememberSaveable { mutableStateOf("") }
    var packageName by rememberSaveable { mutableStateOf("") }
    var language by rememberSaveable { mutableStateOf("bash") }
    var script by rememberSaveable { mutableStateOf("") }
    var copiedName by rememberSaveable { mutableStateOf<String?>(null) }
    var validationError by rememberSaveable { mutableStateOf(false) }
    var saveErrorMessage by rememberSaveable { mutableStateOf<String?>(null) }
    var expandedSnippetId by rememberSaveable { mutableStateOf<String?>(null) }
    var selectedPackage by rememberSaveable { mutableStateOf<String?>(null) }
    var deleteSnippetId by rememberSaveable { mutableStateOf<String?>(null) }
    var newPackageDialogVisible by rememberSaveable { mutableStateOf(false) }
    var newPackageName by rememberSaveable { mutableStateOf("") }
    var newPackageValidationError by rememberSaveable { mutableStateOf(false) }

    val packageSummaries = snippets
        .map { it.packageName.trim() }
        .filter { it.isNotEmpty() }
        .distinctBy { it.lowercase() }
        .sortedBy { it.lowercase() }
        .map { packageValue ->
            packageValue to snippets.count {
                it.packageName.equals(packageValue, ignoreCase = true)
            }
        }
    val visibleSnippets = snippets
        .filter { snippet ->
            selectedPackage == null ||
                snippet.packageName.equals(selectedPackage, ignoreCase = true)
        }
        .sortedBy { it.description.lowercase() }
    val deleteSnippet = snippets.firstOrNull { it.id == deleteSnippetId }

    fun openNew() {
        editingId = null
        description = ""
        packageName = selectedPackage.orEmpty()
        language = "bash"
        script = ""
        validationError = false
        onEditorVisibleChange(true)
    }

    fun openEdit(snippet: SnippetUi) {
        editingId = snippet.id
        description = snippet.description
        packageName = snippet.packageName
        language = snippet.language.ifBlank { "bash" }
        script = snippet.script
        validationError = false
        onEditorVisibleChange(true)
    }

    Box(modifier = modifier) {
        SharedTransitionLayout(modifier = Modifier.fillMaxSize()) {
        Box(modifier = Modifier.fillMaxSize()) {
        AnimatedContent(
            targetState = editorVisible,
            modifier = Modifier.fillMaxSize(),
            transitionSpec = {
                fadeIn(
                    animationSpec = tween(
                        durationMillis = 150,
                        delayMillis = 75,
                        easing = FastOutSlowInEasing
                    )
                ) togetherWith fadeOut(
                    animationSpec = tween(
                        durationMillis = 75,
                        easing = FastOutSlowInEasing
                    )
                )
            },
            label = "snippet-container-transform"
        ) { showEditor ->
            Column(modifier = Modifier.fillMaxSize()) {
            if (!showEditor) {
                MiaominalSearchTopBar(
                    placeholder = stringResource(R.string.snippet_search),
                    modifier = Modifier.fillMaxWidth(),
                    collapsedWindowInsets = WindowInsets(0),
                    initialQuery = initialSearchQuery
                ) { query, collapse ->
                    SnippetSearchResults(
                        query = query,
                        snippets = snippets,
                        onCopy = { snippet ->
                            clipboardManager.setText(AnnotatedString(snippet.script))
                            copiedName = snippet.description
                        },
                        onEdit = { snippet ->
                            collapse()
                            openEdit(snippet)
                        },
                        onDelete = { snippet ->
                            collapse()
                            deleteSnippetId = snippet.id
                        }
                    )
                }
            }
            LazyColumn(
                modifier = Modifier.weight(1f),
                contentPadding = PaddingValues(
                    start = 16.dp,
                    top = 16.dp,
                    end = 16.dp,
                    bottom = if (showEditor) 16.dp else 96.dp
                ),
                verticalArrangement = Arrangement.spacedBy(16.dp)
            ) {
        item { PageSubtitle(text = stringResource(R.string.tool_snippets_desc)) }

        if (showEditor) {
            item {
                SectionTitle(
                    title = stringResource(
                        if (editingId == null) R.string.snippet_add else R.string.snippet_edit
                    )
                )
            }
            item {
                val editorSharedKey = editingId?.let { snippetId ->
                    "snippet-card-$snippetId"
                } ?: "snippet-new"
                val editorCardModifier = Modifier.sharedBounds(
                        sharedContentState = rememberSharedContentState(key = editorSharedKey),
                        animatedVisibilityScope = this@AnimatedContent,
                        enter = fadeIn(
                            animationSpec = tween(
                                durationMillis = 120,
                                delayMillis = 90
                            )
                        ),
                        exit = fadeOut(animationSpec = tween(durationMillis = 90)),
                        boundsTransform = { _, _ ->
                            tween(
                                durationMillis = 350,
                                easing = FastOutSlowInEasing
                            )
                        },
                        resizeMode = SharedTransitionScope.ResizeMode.scaleToBounds()
                    )
                Card(
                    modifier = editorCardModifier,
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
                    )
                ) {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp)
                    ) {
                        OutlinedTextField(
                            value = description,
                            onValueChange = { description = it },
                            modifier = Modifier.fillMaxWidth(),
                            singleLine = true,
                            label = { Text(stringResource(R.string.snippet_description)) }
                        )
                        SnippetPackageSelector(
                            packages = packageSummaries.map { it.first },
                            value = packageName,
                            onValueChange = { packageName = it },
                            onAddPackage = {
                                newPackageName = ""
                                newPackageValidationError = false
                                newPackageDialogVisible = true
                            }
                        )
                        OutlinedTextField(
                            value = script,
                            onValueChange = { script = it },
                            modifier = Modifier.fillMaxWidth(),
                            minLines = 8,
                            label = { Text(stringResource(R.string.snippet_script)) },
                            textStyle = MaterialTheme.typography.bodyLarge.copy(
                                fontFamily = FontFamily.Monospace
                            )
                        )
                        if (validationError) {
                            Text(
                                stringResource(R.string.tool_form_required),
                                color = MaterialTheme.colorScheme.error,
                                style = MaterialTheme.typography.bodyMedium
                            )
                        }
                        saveErrorMessage?.let { message ->
                            Text(
                                message,
                                color = MaterialTheme.colorScheme.error,
                                style = MaterialTheme.typography.bodyMedium
                            )
                        }
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(
                                8.dp,
                                Alignment.End
                            ),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            TextButton(onClick = { onEditorVisibleChange(false) }) {
                                Text(stringResource(R.string.tool_action_cancel))
                            }
                            Button(
                                onClick = {
                                    val valid = description.isNotBlank() && script.isNotBlank()
                                    validationError = !valid
                                    if (valid) {
                                        val saved = SnippetUi(
                                            id = editingId ?: "snippet-${System.currentTimeMillis()}",
                                            description = description.trim(),
                                            packageName = packageName.trim(),
                                            language = language.ifBlank { "bash" },
                                            script = script
                                        )
                                        scope.launch {
                                            val result = repository.saveSnippet(
                                                SnippetRecord(
                                                    id = saved.id,
                                                    description = saved.description,
                                                    packageName = saved.packageName,
                                                    language = saved.language,
                                                    script = saved.script
                                                )
                                            )
                                            if (result.isSuccess) {
                                                onEditorVisibleChange(false)
                                            } else {
                                                saveErrorMessage = result.exceptionOrNull()
                                                    ?.message ?: uiString(R.string.ui_could_not_save)
                                            }
                                        }
                                    }
                                }
                            ) {
                                Text(stringResource(R.string.tool_action_save))
                            }
                        }
                    }
                }
            }
        } else {
            if (packageSummaries.isNotEmpty()) {
                item {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        SectionTitle(title = stringResource(R.string.snippet_packages))
                        LazyRow(
                            horizontalArrangement = Arrangement.spacedBy(8.dp)
                        ) {
                            item(key = "all-packages") {
                                FilterChip(
                                    selected = selectedPackage == null,
                                    onClick = { selectedPackage = null },
                                    label = { Text(stringResource(R.string.snippet_package_all)) }
                                )
                            }
                            items(
                                items = packageSummaries,
                                key = { (packageValue, _) -> packageValue.lowercase() }
                            ) { (packageValue, count) ->
                                FilterChip(
                                    selected = selectedPackage?.equals(
                                        packageValue,
                                        ignoreCase = true
                                    ) == true,
                                    onClick = {
                                        selectedPackage = if (selectedPackage?.equals(
                                                packageValue,
                                                ignoreCase = true
                                            ) == true
                                        ) {
                                            null
                                        } else {
                                            packageValue
                                        }
                                    },
                                    label = {
                                        Text(
                                            stringResource(
                                                R.string.snippet_package_count,
                                                packageValue,
                                                count
                                            )
                                        )
                                    }
                                )
                            }
                        }
                    }
                }
            }
            copiedName?.let { copied ->
                item {
                    StatusCard(
                        icon = Icons.Outlined.ContentCopy,
                        title = stringResource(R.string.tool_action_copy),
                        body = stringResource(R.string.snippet_copied, copied)
                    )
                }
            }
            if (visibleSnippets.isEmpty()) {
                item {
                    PageEmptyState(
                        title = stringResource(R.string.page_empty_title),
                        modifier = Modifier.fillParentMaxHeight(0.65f),
                        icon = Icons.Outlined.Code,
                        text = stringResource(R.string.snippet_empty)
                    )
                }
            } else {
                items(visibleSnippets, key = { it.id }) { snippet ->
                    SnippetCard(
                        snippet = snippet,
                        modifier = Modifier.sharedBounds(
                            sharedContentState = rememberSharedContentState(
                                key = "snippet-card-${snippet.id}"
                            ),
                            animatedVisibilityScope = this@AnimatedContent,
                            enter = fadeIn(
                                animationSpec = tween(
                                    durationMillis = 120,
                                    delayMillis = 90
                                )
                            ),
                            exit = fadeOut(animationSpec = tween(durationMillis = 90)),
                            boundsTransform = { _, _ ->
                                tween(
                                    durationMillis = 350,
                                    easing = FastOutSlowInEasing
                                )
                            },
                            resizeMode = SharedTransitionScope.ResizeMode.scaleToBounds()
                        ),
                        expanded = expandedSnippetId == snippet.id,
                        onExpandedChange = {
                            expandedSnippetId = if (expandedSnippetId == snippet.id) {
                                null
                            } else {
                                snippet.id
                            }
                        },
                        onCopy = {
                            clipboardManager.setText(AnnotatedString(snippet.script))
                            copiedName = snippet.description
                        },
                        onEdit = { openEdit(snippet) },
                        onDelete = {
                            deleteSnippetId = snippet.id
                        }
                    )
                }
            }
        }
            }
        }
            }
        AnimatedVisibility(
            visible = !editorVisible,
            modifier = Modifier
                .align(Alignment.BottomEnd)
                .padding(16.dp),
            enter = fadeIn(animationSpec = tween(180, delayMillis = 100)) +
                scaleIn(animationSpec = tween(240, easing = ForwardPageEasing)),
            exit = fadeOut(animationSpec = tween(120)) +
                scaleOut(animationSpec = tween(180, easing = FastOutSlowInEasing))
        ) fabVisibility@ {
            FloatingActionButton(
                onClick = ::openNew,
                modifier = Modifier.sharedBounds(
                    sharedContentState = rememberSharedContentState(key = "snippet-new"),
                    animatedVisibilityScope = this@fabVisibility,
                    enter = fadeIn(
                        animationSpec = tween(
                            durationMillis = 120,
                            delayMillis = 90
                        )
                    ),
                    exit = fadeOut(animationSpec = tween(durationMillis = 90)),
                    boundsTransform = { _, _ ->
                        tween(
                            durationMillis = 350,
                            easing = FastOutSlowInEasing
                        )
                    },
                    resizeMode = SharedTransitionScope.ResizeMode.scaleToBounds()
                ),
                containerColor = MaterialTheme.colorScheme.primaryContainer,
                contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                shape = MaterialTheme.shapes.large
            ) {
                Icon(
                    imageVector = Icons.Outlined.Add,
                    contentDescription = stringResource(R.string.snippet_add)
                )
            }
        }
        }
        }
        deleteSnippet?.let { snippet ->
            AlertDialog(
                onDismissRequest = { deleteSnippetId = null },
                icon = { Icon(Icons.Outlined.Delete, contentDescription = null) },
                title = { Text(stringResource(R.string.snippet_delete_title)) },
                text = {
                    Text(
                        stringResource(
                            R.string.snippet_delete_message,
                            snippet.description
                        )
                    )
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            if (expandedSnippetId == snippet.id) expandedSnippetId = null
                            scope.launch { repository.deleteSnippet(snippet.id) }
                            if (selectedPackage != null && storedSnippets.none { candidate ->
                                    candidate.id != snippet.id &&
                                        candidate.packageName.equals(selectedPackage, ignoreCase = true)
                                }
                            ) {
                                selectedPackage = null
                            }
                            deleteSnippetId = null
                        },
                        colors = ButtonDefaults.textButtonColors(
                            contentColor = MaterialTheme.colorScheme.error
                        )
                    ) {
                        Text(stringResource(R.string.tool_action_delete))
                    }
                },
                dismissButton = {
                    TextButton(onClick = { deleteSnippetId = null }) {
                        Text(stringResource(R.string.tool_action_cancel))
                    }
                }
            )
        }
        if (newPackageDialogVisible) {
            AlertDialog(
                onDismissRequest = {
                    newPackageDialogVisible = false
                    newPackageValidationError = false
                },
                icon = { Icon(Icons.Outlined.Add, contentDescription = null) },
                title = { Text(stringResource(R.string.snippet_new_package_title)) },
                text = {
                    OutlinedTextField(
                        value = newPackageName,
                        onValueChange = {
                            newPackageName = it
                            if (it.isNotBlank()) newPackageValidationError = false
                        },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        label = { Text(stringResource(R.string.snippet_new_package_name)) },
                        isError = newPackageValidationError,
                        supportingText = if (newPackageValidationError) {
                            {
                                Text(stringResource(R.string.tool_form_required))
                            }
                        } else {
                            null
                        }
                    )
                },
                confirmButton = {
                    Button(
                        onClick = {
                            val normalizedName = newPackageName.trim()
                            if (normalizedName.isEmpty()) {
                                newPackageValidationError = true
                            } else {
                                packageName = packageSummaries
                                    .map { it.first }
                                    .firstOrNull {
                                        it.equals(normalizedName, ignoreCase = true)
                                    }
                                    ?: normalizedName
                                newPackageDialogVisible = false
                                newPackageValidationError = false
                            }
                        }
                    ) {
                        Text(stringResource(R.string.snippet_new_package_confirm))
                    }
                },
                dismissButton = {
                    TextButton(
                        onClick = {
                            newPackageDialogVisible = false
                            newPackageValidationError = false
                        }
                    ) {
                        Text(stringResource(R.string.tool_action_cancel))
                    }
                }
            )
        }
    }
}

@Composable
private fun SnippetSearchResults(
    query: String,
    snippets: List<SnippetUi>,
    onCopy: (SnippetUi) -> Unit,
    onEdit: (SnippetUi) -> Unit,
    onDelete: (SnippetUi) -> Unit
) {
    var expandedSnippetId by rememberSaveable { mutableStateOf<String?>(null) }
    val snippetSnapshot = snippets.toList()
    val visibleSnippets = remember(query, snippetSnapshot) {
        if (query.isEmpty()) {
            snippetSnapshot
        } else {
            snippetSnapshot.filter { snippet ->
                snippet.description.contains(query, ignoreCase = true) ||
                    snippet.packageName.contains(query, ignoreCase = true) ||
                    snippet.language.contains(query, ignoreCase = true) ||
                    snippet.script.contains(query, ignoreCase = true)
            }
        }
    }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(8.dp, 8.dp, 8.dp, 16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        if (visibleSnippets.isEmpty()) {
            item {
                PageEmptyState(
                        title = stringResource(R.string.page_empty_title),
                        modifier = Modifier.fillParentMaxHeight(0.65f),
                    icon = Icons.Outlined.Code,
                    text = stringResource(R.string.snippet_empty)
                )
            }
        } else {
            items(visibleSnippets, key = { it.id }) { snippet ->
                SnippetCard(
                    snippet = snippet,
                    expanded = expandedSnippetId == snippet.id,
                    onExpandedChange = {
                        expandedSnippetId = if (expandedSnippetId == snippet.id) {
                            null
                        } else {
                            snippet.id
                        }
                    },
                    onCopy = { onCopy(snippet) },
                    onEdit = { onEdit(snippet) },
                    onDelete = { onDelete(snippet) }
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SnippetPackageSelector(
    packages: List<String>,
    value: String,
    onValueChange: (String) -> Unit,
    onAddPackage: () -> Unit
) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        ExposedDropdownMenuBox(
            expanded = expanded,
            onExpandedChange = { expanded = it },
            modifier = Modifier.weight(1f)
        ) {
            OutlinedTextField(
                value = value.ifBlank { stringResource(R.string.snippet_no_package) },
                onValueChange = {},
                modifier = Modifier
                    .fillMaxWidth()
                    .menuAnchor(ExposedDropdownMenuAnchorType.PrimaryNotEditable),
                readOnly = true,
                singleLine = true,
                label = { Text(stringResource(R.string.snippet_package)) },
                trailingIcon = {
                    ExposedDropdownMenuDefaults.TrailingIcon(expanded = expanded)
                }
            )
            ExposedDropdownMenu(
                expanded = expanded,
                onDismissRequest = { expanded = false }
            ) {
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.snippet_no_package)) },
                    onClick = {
                        onValueChange("")
                        expanded = false
                    }
                )
                packages.forEach { packageValue ->
                    DropdownMenuItem(
                        text = { Text(packageValue) },
                        onClick = {
                            onValueChange(packageValue)
                            expanded = false
                        }
                    )
                }
            }
        }
        FilledTonalIconButton(onClick = onAddPackage) {
            Icon(
                imageVector = Icons.Outlined.Add,
                contentDescription = stringResource(R.string.snippet_new_package)
            )
        }
    }
}

@Composable
private fun SnippetCard(
    snippet: SnippetUi,
    modifier: Modifier = Modifier,
    expanded: Boolean,
    onExpandedChange: () -> Unit,
    onCopy: () -> Unit,
    onEdit: () -> Unit,
    onDelete: () -> Unit
) {
    val preview = snippet.script
        .lineSequence()
        .map(String::trim)
        .firstOrNull(String::isNotEmpty)
        .orEmpty()
    val chevronRotation by animateFloatAsState(
        targetValue = if (expanded) 180f else 0f,
        animationSpec = tween(durationMillis = 240, easing = FastOutSlowInEasing),
        label = "snippet-chevron"
    )
    Card(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
        )
    ) {
        Column {
            Row(
                modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 4.dp, bottom = 12.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                ToolIconTile(Icons.Outlined.Code)
                Spacer(Modifier.width(12.dp))
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    Text(
                        text = snippet.description,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                    Text(
                        text = preview,
                        style = MaterialTheme.typography.bodyMedium.copy(
                            fontFamily = FontFamily.Monospace
                        ),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                }
                IconButton(onClick = onExpandedChange) {
                    Icon(
                        imageVector = Icons.Filled.KeyboardArrowDown,
                        contentDescription = stringResource(
                            if (expanded) R.string.action_collapse else R.string.action_expand
                        ),
                        modifier = Modifier.graphicsLayer { rotationZ = chevronRotation }
                    )
                }
            }
            AnimatedVisibility(
                visible = expanded,
                enter = expandVertically(
                    animationSpec = tween(280, easing = FastOutSlowInEasing),
                    expandFrom = Alignment.Top
                ) + fadeIn(tween(180, delayMillis = 40)),
                exit = shrinkVertically(
                    animationSpec = tween(240, easing = FastOutSlowInEasing),
                    shrinkTowards = Alignment.Top
                ) + fadeOut(tween(160))
            ) {
                Column(
                    modifier = Modifier.padding(start = 16.dp, end = 12.dp, bottom = 12.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    Text(
                        text = stringResource(
                            R.string.snippet_package_value,
                            snippet.packageName.ifBlank {
                                stringResource(R.string.snippet_no_package)
                            }
                        ),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Surface(
                        modifier = Modifier.fillMaxWidth(),
                        color = MaterialTheme.colorScheme.surfaceContainer,
                        shape = MaterialTheme.shapes.medium
                    ) {
                        Text(
                            text = snippet.script,
                            modifier = Modifier.padding(12.dp),
                            style = MaterialTheme.typography.bodyMedium.copy(
                                fontFamily = FontFamily.Monospace
                            )
                        )
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Button(onClick = onCopy) {
                            Icon(Icons.Outlined.ContentCopy, contentDescription = null)
                            Spacer(Modifier.width(8.dp))
                            Text(stringResource(R.string.tool_action_copy))
                        }
                        Spacer(Modifier.weight(1f))
                        IconButton(onClick = onEdit) {
                            Icon(
                                Icons.Outlined.Edit,
                                contentDescription = stringResource(R.string.tool_action_edit)
                            )
                        }
                        IconButton(onClick = onDelete) {
                            Icon(
                                Icons.Outlined.Delete,
                                contentDescription = stringResource(R.string.tool_action_delete)
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun StatusCard(
    icon: ImageVector,
    title: String,
    body: String
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.secondaryContainer,
            contentColor = MaterialTheme.colorScheme.onSecondaryContainer
        )
    ) {
        ListItem(
            headlineContent = {
                Text(title, style = MaterialTheme.typography.titleMedium)
            },
            supportingContent = {
                Text(body, style = MaterialTheme.typography.bodyMedium)
            },
            leadingContent = {
                Icon(icon, contentDescription = null)
            },
            colors = ListItemDefaults.colors(
                containerColor = Color.Transparent,
                headlineColor = MaterialTheme.colorScheme.onSecondaryContainer,
                supportingColor = MaterialTheme.colorScheme.onSecondaryContainer,
                leadingIconColor = MaterialTheme.colorScheme.onSecondaryContainer
            )
        )
    }
}
