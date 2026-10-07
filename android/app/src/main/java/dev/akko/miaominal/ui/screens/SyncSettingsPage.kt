package dev.akko.miaominal.ui.screens

import android.text.format.DateUtils
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.akko.miaominal.R
import dev.akko.miaominal.data.*
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.AdaptiveWorkspace
import dev.akko.miaominal.ui.components.HintIcon as Icon
import dev.akko.miaominal.ui.components.SectionTitle
import dev.akko.miaominal.ui.model.*
import dev.akko.miaominal.ui.theme.ExpressiveMotion
import dev.akko.miaominal.ui.theme.UiSpacing
import dev.akko.miaominal.ui.theme.syncDownloadButtonColors
import dev.akko.miaominal.ui.uiString
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
internal fun SyncSettingsPage(modifier: Modifier = Modifier, focusAnchor: String? = null) {
    val repository = LocalMiaominalRepository.current
    val sync by repository.sync.collectAsState()
    val config = sync.config
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    var connectionEditor by rememberSaveable { mutableStateOf(false) }
    var passphraseEditor by rememberSaveable { mutableStateOf(false) }
    var confirmation by rememberSaveable { mutableStateOf<String?>(null) }
    var conflictChoice by rememberSaveable(sync.pendingConflict) { mutableStateOf<Boolean?>(null) }
    var operation by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var advanced by rememberSaveable { mutableStateOf(false) }
    val busy = !sync.loaded || operation != null || sync.phase.isRunning
    val connectionReady = syncConnectionReady(config)
    val ready = connectionReady && config.hasPassphrase
    val actionsEnabled = ready && !busy && !sync.vaultLocked && sync.pendingConflict == null
    val downloadEnabled = actionsEnabled && (config.provider != SyncProviderModel.Gist || config.gistId.isNotBlank())
    val clipboard = LocalClipboardManager.current
    val listState = rememberLazyListState()
    SettingsAnchorScroll(listState, focusAnchor)
    LaunchedEffect(focusAnchor, sync.capability.state) {
        if (focusAnchor in listOf("sync-automatic", "sync-capabilities", "sync-unsafe") ||
            sync.capability.state.equals("UNSUPPORTED", true)) advanced = true
    }

    fun run(label: String, success: String? = null, action: suspend () -> Result<Unit>) {
        if (operation != null || !repository.sync.value.loaded || repository.sync.value.phase.isRunning) return
        operation = label
        error = null
        scope.launch {
            val result = try { syncRequest(action) } finally { operation = null }
            result.fold(
                onSuccess = { if (success != null) snackbar.showSnackbar(success) },
                onFailure = { error = describeSyncError(it); listState.animateScrollToItem(0) }
            )
        }
    }

    fun showSetup() {
        if (!connectionReady) connectionEditor = true else passphraseEditor = true
    }

    Box(modifier) {
        LazyColumn(
            state = listState,
            modifier = Modifier.fillMaxSize().testTag("sync-overview"),
            contentPadding = PaddingValues(UiSpacing.sm),
            verticalArrangement = Arrangement.spacedBy(UiSpacing.md)
        ) {
            item(key = "sync-status") {
                val needsAttention = sync.pendingConflict != null || sync.lastFailure != null || sync.status is SyncStatusModel.Failed
                val title = if (busy && sync.loaded) operation ?: syncPhaseLabel(sync.phase)
                    else syncOverviewTitle(sync)?.let { stringResource(it) } ?: syncPhaseLabel(sync.phase)
                SyncCard(
                    color = if (needsAttention) MaterialTheme.colorScheme.errorContainer else MaterialTheme.colorScheme.primaryContainer,
                    contentColor = if (needsAttention) MaterialTheme.colorScheme.onErrorContainer else MaterialTheme.colorScheme.onPrimaryContainer
                ) {
                    Icon(if (needsAttention) Icons.Outlined.ErrorOutline else Icons.Outlined.CloudSync, null, Modifier.size(32.dp))
                    Text(title, style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.SemiBold,
                        modifier = Modifier.testTag("sync-overview-title").semantics { heading(); liveRegion = LiveRegionMode.Polite })
                    Text(stringResource(R.string.sync_mobile_scope), style = MaterialTheme.typography.bodyMedium)
                    if (ready && !busy) Text(stringResource(if (config.autoSyncEnabled)
                        R.string.sync_mobile_auto_on else R.string.sync_mobile_auto_off),
                        style = MaterialTheme.typography.bodyMedium, modifier = Modifier.testTag("sync-auto-summary"))
                    if (config.lastSyncAtMillis > 0) {
                        Text(stringResource(R.string.sync_mobile_last_synced, DateUtils.getRelativeTimeSpanString(
                            config.lastSyncAtMillis, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString()),
                            style = MaterialTheme.typography.bodySmall)
                    }
                    if (busy) {
                        LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                        if (sync.phase.isRunning) OutlinedButton(
                            onClick = { repository.syncCancel() }, modifier = Modifier.fillMaxWidth(),
                            enabled = sync.phase != SyncPhaseModel.Cancelling
                        ) { Text(stringResource(R.string.action_cancel)) }
                    } else if (!ready) {
                        Text(stringResource(if (!connectionReady) R.string.sync_mobile_setup_destination_hint else R.string.sync_mobile_setup_encryption_hint))
                        Button(onClick = ::showSetup, modifier = Modifier.fillMaxWidth().testTag("sync-setup")) {
                            Text(stringResource(if (!connectionReady) R.string.sync_mobile_configure_destination else R.string.desktop_sync_passphrase_set))
                        }
                    } else {
                        Button(
                            enabled = actionsEnabled, modifier = Modifier.fillMaxWidth().testTag("sync-upload"),
                            onClick = { run(uiString(R.string.ui_pushing), uiString(R.string.sync_mobile_uploaded)) { repository.syncPush(manual = true) } }
                        ) {
                            Icon(Icons.Outlined.CloudUpload, null); Spacer(Modifier.width(UiSpacing.xs))
                            Text(stringResource(R.string.sync_mobile_upload))
                        }
                        FilledTonalButton(
                            enabled = downloadEnabled,
                            colors = syncDownloadButtonColors(MaterialTheme.colorScheme),
                            border = BorderStroke(1.dp, if (downloadEnabled) MaterialTheme.colorScheme.outline else MaterialTheme.colorScheme.outlineVariant),
                            modifier = Modifier.fillMaxWidth().testTag("sync-download"),
                            onClick = { confirmation = "download" }
                        ) {
                            Icon(Icons.Outlined.CloudDownload, null); Spacer(Modifier.width(UiSpacing.xs))
                            Text(stringResource(R.string.sync_mobile_download))
                        }
                        if (config.provider == SyncProviderModel.Gist && config.gistId.isBlank()) {
                            Text(stringResource(R.string.sync_mobile_gist_create_hint), style = MaterialTheme.typography.bodySmall)
                        }
                    }
                }
                if (sync.vaultLocked) SyncMessage(uiString(R.string.ui_vault_locked_automatic_sync_is_paused_until_you_unlock_it))
                error?.let { SyncMessage(it) }
                sync.lastFailure?.let { failure ->
                    var details by rememberSaveable(failure) { mutableStateOf(false) }
                    SyncMessage(uiString(R.string.ui_sync_failed, syncFailureReason(failure.kind))) {
                        syncFailureAdvice(failure.kind)?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
                        TextButton(onClick = { details = !details }) { Text(uiString(R.string.ui_view_details)) }
                        if (details) {
                            SelectionContainer { Text(failure.detail, style = MaterialTheme.typography.bodySmall) }
                            TextButton(onClick = { clipboard.setText(AnnotatedString(failure.detail)) }) {
                                Text(stringResource(R.string.sync_mobile_copy_error))
                            }
                        }
                    }
                }
            }
            sync.pendingConflict?.let { conflict ->
                item(key = "sync-conflict") {
                    SyncSection(uiString(R.string.ui_resolve_conflict), focusAnchor == "sync-conflict") {
                        Text(syncInterventionLabel(conflict.reason), style = MaterialTheme.typography.titleMedium)
                        conflict.remoteAtMillis?.let { Text(stringResource(R.string.sync_mobile_remote_time, formatSyncTimestamp(it))) }
                        Text(uiString(R.string.ui_sync_will_continue_after_you_choose_a_version_neither_side_will_b), style = MaterialTheme.typography.bodyMedium)
                        Button(onClick = { conflictChoice = true }, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
                            Text(uiString(R.string.ui_keep_local_overwrite_remote))
                        }
                        OutlinedButton(onClick = { conflictChoice = false }, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
                            Text(uiString(R.string.ui_keep_remote_overwrite_local))
                        }
                    }
                }
            }
            item(key = "sync-provider") {
                SyncSection(stringResource(R.string.sync_mobile_destination), focusAnchor == "sync-provider") {
                    SyncEditRow(
                        icon = Icons.Outlined.Dns,
                        title = providerLabel(config.provider),
                        body = when (config.provider) {
                            SyncProviderModel.None -> stringResource(R.string.sync_mobile_destination_empty)
                            SyncProviderModel.Gist -> config.gistId.ifBlank { stringResource(R.string.sync_mobile_gist_new) }
                            SyncProviderModel.WebDav -> config.webdavUrl.ifBlank { stringResource(R.string.sync_mobile_destination_empty) }
                        }, enabled = !busy, tag = "sync-edit-connection", onClick = { connectionEditor = true }
                    )
                    if (config.provider != SyncProviderModel.None) {
                        Text(stringResource(if (connectionReady) R.string.sync_mobile_credentials_saved else R.string.sync_mobile_credentials_missing),
                            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            item(key = "sync-encryption") {
                SyncSection(stringResource(R.string.desktop_sync_encryption_group), focusAnchor == "sync-encryption") {
                    SyncEditRow(Icons.Outlined.Lock,
                        stringResource(if (config.hasPassphrase) R.string.sync_mobile_encryption_ready else R.string.sync_mobile_encryption_missing),
                        stringResource(R.string.sync_mobile_encryption_hint), !busy, "sync-edit-encryption", { passphraseEditor = true })
                }
            }
            item(key = "sync-automatic") {
                SyncSection(uiString(R.string.ui_automatic_sync), focusAnchor == "sync-automatic") {
                    SyncSwitchRow(
                        title = stringResource(R.string.sync_mobile_auto_title),
                        body = stringResource(R.string.sync_mobile_auto_hint), checked = config.autoSyncEnabled,
                        enabled = !busy && (ready || config.autoSyncEnabled), tag = "sync-auto",
                        onChange = { enabled -> run(uiString(R.string.ui_automatic_sync)) { repository.syncSetAutoSync(enabled) } }
                    )
                    if (!ready) Text(stringResource(R.string.sync_mobile_auto_requires_setup), style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            item(key = "sync-capabilities") {
                Column(verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                    TextButton(onClick = { advanced = !advanced }, modifier = Modifier.fillMaxWidth().testTag("sync-advanced")) {
                        Text(stringResource(R.string.sync_mobile_advanced), modifier = Modifier.weight(1f))
                        Icon(if (advanced) Icons.Outlined.ExpandLess else Icons.Outlined.ExpandMore, null)
                    }
                    AnimatedVisibility(advanced,
                        enter = expandVertically(ExpressiveMotion.stateChange()), exit = shrinkVertically(ExpressiveMotion.stateChange())) {
                        SyncCard {
                            Text(stringResource(R.string.sync_mobile_connection_test), style = MaterialTheme.typography.titleMedium,
                                modifier = Modifier.semantics { heading() })
                            Text(syncCapabilitySummary(sync.capability), style = MaterialTheme.typography.bodyMedium)
                            if (sync.capability.cleanupPending) Text(stringResource(R.string.sync_mobile_cleanup_pending),
                                color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
                            if (sync.capability.reason != null || sync.capability.httpStatus != null) {
                                var showDetail by rememberSaveable { mutableStateOf(false) }
                                TextButton(onClick = { showDetail = !showDetail }) { Text(uiString(R.string.ui_view_details)) }
                                if (showDetail) SelectionContainer {
                                    Text(listOfNotNull(sync.capability.reason, sync.capability.step.takeIf { it.isNotBlank() },
                                        sync.capability.httpStatus?.let { "HTTP $it" }).joinToString(" · "), style = MaterialTheme.typography.bodySmall)
                                }
                            }
                            Text(stringResource(R.string.sync_mobile_capability_hint), style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant)
                            OutlinedButton(
                                onClick = { run(uiString(R.string.ui_checking_remote_capabilities), uiString(R.string.sync_mobile_check_finished)) { repository.syncCheckCapability() } },
                                enabled = connectionReady && !busy && !sync.vaultLocked,
                                modifier = Modifier.fillMaxWidth().testTag("sync-check")
                            ) { Text(uiString(R.string.ui_check_remote_capabilities)) }
                            if (config.provider == SyncProviderModel.WebDav && (config.webdavUnsafeWriteConsent ||
                                (sync.capability.state.equals("UNSUPPORTED", true) && sync.capability.reason in listOf("VERSION_UNAVAILABLE", "CONDITIONAL_READ", "CONDITIONAL_WRITE")))) {
                                HorizontalDivider()
                                SyncSwitchRow(uiString(R.string.ui_allow_unconditional_writes_last_writer_wins),
                                    uiString(R.string.ui_remote_storage_cannot_guarantee_safe_writes_sync_still_compares_c),
                                    config.webdavUnsafeWriteConsent, !busy, "sync-unsafe", { enabled ->
                                        if (enabled) confirmation = "unsafe" else run(uiString(R.string.ui_automatic_sync)) {
                                            repository.updateSyncConfig(SyncConfigEdit(webdavUnsafeWriteConsent = false))
                                        }
                                    })
                            }
                            if (config.provider != SyncProviderModel.None) {
                                TextButton(onClick = { confirmation = "disable" }, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
                                    Text(stringResource(R.string.sync_mobile_disable), color = MaterialTheme.colorScheme.error)
                                }
                            }
                        }
                    }
                }
            }
        }
        SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).padding(UiSpacing.sm))
    }

    conflictChoice?.let { keepLocal ->
        if (sync.pendingConflict != null) SyncConfirmation(
            title = uiString(if (keepLocal) R.string.ui_overwrite_remote_configuration else R.string.ui_overwrite_local_configuration),
            body = uiString(if (keepLocal) R.string.ui_replace_the_remote_configuration_with_the_local_configuration else R.string.ui_replace_the_local_configuration_with_the_remote_configuration),
            enabled = !busy, onDismiss = { conflictChoice = null }, onConfirm = {
                conflictChoice = null
                run(uiString(R.string.ui_resolve_conflict)) { repository.syncResolveConflict(keepLocal) }
            })
    }
    confirmation?.let { request ->
        SyncConfirmation(
            title = stringResource(when (request) {
                "download" -> R.string.sync_mobile_download_title
                "unsafe" -> R.string.sync_mobile_unsafe_title
                else -> R.string.sync_mobile_disable_title
            }),
            body = stringResource(when (request) {
                "download" -> R.string.sync_mobile_download_warning
                "unsafe" -> R.string.sync_mobile_unsafe_warning
                else -> R.string.sync_mobile_disable_warning
            }), enabled = !busy, onDismiss = { confirmation = null }, onConfirm = {
                confirmation = null
                when (request) {
                    "download" -> run(uiString(R.string.ui_pulling), uiString(R.string.sync_mobile_downloaded)) { repository.syncPull(force = true) }
                    "unsafe" -> run(uiString(R.string.ui_automatic_sync)) { repository.updateSyncConfig(SyncConfigEdit(webdavUnsafeWriteConsent = true)) }
                    else -> run(uiString(R.string.sync_mobile_disable), uiString(R.string.sync_mobile_disabled)) {
                        repository.syncSetAutoSync(false).fold(
                            onSuccess = { repository.updateSyncConfig(SyncConfigEdit(provider = SyncProviderModel.None)) },
                            onFailure = { Result.failure(it) }
                        )
                    }
                }
            })
    }
    if (connectionEditor) SyncConnectionEditor(onDismiss = { connectionEditor = false }, onSaved = { changed ->
        connectionEditor = false
        scope.launch { snackbar.showSnackbar(uiString(if (changed) R.string.sync_mobile_saved_destination_changed else R.string.sync_mobile_connection_saved)) }
    })
    if (passphraseEditor) SyncPassphraseEditor(onDismiss = { passphraseEditor = false }, onSaved = {
        passphraseEditor = false
        scope.launch { snackbar.showSnackbar(uiString(if (repository.sync.value.config.hasPassphrase) R.string.sync_mobile_passphrase_saved else R.string.sync_mobile_passphrase_cleared)) }
    })
}

@Composable
private fun providerLabel(provider: SyncProviderModel): String = stringResource(when (provider) {
    SyncProviderModel.None -> R.string.sync_mobile_not_configured
    SyncProviderModel.Gist -> R.string.settings_sync_gist
    SyncProviderModel.WebDav -> R.string.settings_sync_webdav
})

private fun syncCapabilitySummary(capability: SyncCapabilityModel): String = when (capability.state.uppercase()) {
    "SUPPORTED" -> uiString(R.string.ui_supported)
    "UNCHECKED" -> uiString(R.string.ui_not_checked)
    "CHECKING" -> uiString(R.string.ui_checking)
    else -> uiString(when (capability.reason?.uppercase()) {
        "VERSION_UNAVAILABLE" -> R.string.sync_mobile_capability_version
        "CONDITIONAL_READ" -> R.string.sync_mobile_capability_read
        "CONDITIONAL_WRITE" -> R.string.sync_mobile_capability_write
        "AUTHENTICATION" -> R.string.ui_remote_credentials_are_invalid_or_expired
        "NETWORK" -> R.string.ui_network_unavailable_or_remote_service_not_responding
        "PERMISSION" -> R.string.sync_mobile_capability_permission
        "REDIRECT" -> R.string.sync_mobile_capability_redirect
        "CONFIGURATION" -> R.string.ui_incomplete_sync_configuration
        "CANCELLED" -> R.string.ui_operation_cancelled
        "CLEANUP" -> R.string.sync_mobile_cleanup_pending
        else -> R.string.sync_mobile_capability_incomplete
    })
}

@Composable
private fun SyncCard(
    color: Color = MaterialTheme.colorScheme.surfaceContainerLowest,
    contentColor: Color = MaterialTheme.colorScheme.onSurface,
    content: @Composable ColumnScope.() -> Unit
) {
    Card(Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.large,
        colors = CardDefaults.cardColors(containerColor = color, contentColor = contentColor)) {
        Column(Modifier.padding(UiSpacing.sm), verticalArrangement = Arrangement.spacedBy(UiSpacing.sm), content = content)
    }
}

@Composable
private fun SyncSection(title: String, focused: Boolean, content: @Composable ColumnScope.() -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
        SectionTitle(title, focused = focused)
        SyncCard(content = content)
    }
}

@Composable
private fun SyncMessage(message: String, content: @Composable ColumnScope.() -> Unit = {}) {
    Surface(color = MaterialTheme.colorScheme.errorContainer, contentColor = MaterialTheme.colorScheme.onErrorContainer,
        shape = MaterialTheme.shapes.large, modifier = Modifier.fillMaxWidth().padding(top = UiSpacing.xs)) {
        Column(Modifier.padding(UiSpacing.sm), verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
            Text(message, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
            content()
        }
    }
}

@Composable
private fun SyncEditRow(icon: ImageVector, title: String, body: String, enabled: Boolean, tag: String, onClick: () -> Unit) {
    Surface(onClick = onClick, enabled = enabled, modifier = Modifier.fillMaxWidth().testTag(tag),
        color = Color.Transparent, shape = MaterialTheme.shapes.medium) {
        Row(Modifier.padding(vertical = UiSpacing.xs), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(UiSpacing.sm)) {
            Icon(icon, null, tint = MaterialTheme.colorScheme.primary)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
                Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                Text(body, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null)
        }
    }
}

@Composable
private fun SyncSwitchRow(title: String, body: String, checked: Boolean, enabled: Boolean, tag: String, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().testTag(tag).toggleable(checked, enabled = enabled, role = Role.Switch, onValueChange = onChange)
        .padding(vertical = UiSpacing.xs), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(UiSpacing.sm)) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            Text(body, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Switch(checked = checked, enabled = enabled, onCheckedChange = null)
    }
}

@Composable
private fun SyncConfirmation(title: String, body: String, enabled: Boolean = true, onDismiss: () -> Unit, onConfirm: () -> Unit) {
    AlertDialog(onDismissRequest = onDismiss, title = { Text(title) }, text = { Text(body) },
        confirmButton = { TextButton(onClick = onConfirm, enabled = enabled) { Text(stringResource(R.string.action_confirm)) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } })
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SyncConnectionEditor(onDismiss: () -> Unit, onSaved: (Boolean) -> Unit) {
    val repository = LocalMiaominalRepository.current
    val sync by repository.sync.collectAsState()
    // Snapshot the form once. Backend refreshes must not overwrite a user's draft.
    val original = remember { sync.config }
    val initialProvider = if (original.provider == SyncProviderModel.None) SyncProviderModel.Gist.name else original.provider.name
    var providerName by rememberSaveable { mutableStateOf(initialProvider) }
    val provider = SyncProviderModel.valueOf(providerName)
    var gistId by rememberSaveable { mutableStateOf(original.gistId) }
    var url by rememberSaveable { mutableStateOf(original.webdavUrl) }
    var username by rememberSaveable { mutableStateOf(original.webdavUsername) }
    // Credentials stay in memory, never in the saved-instance-state bundle.
    var token by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var removeToken by rememberSaveable { mutableStateOf(false) }
    var removePassword by rememberSaveable { mutableStateOf(false) }
    var saving by remember { mutableStateOf(false) }
    var attempted by rememberSaveable { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var discard by remember { mutableStateOf(false) }
    var removeConfirm by remember { mutableStateOf(false) }
    val credential = if (provider == SyncProviderModel.Gist) token else password
    val removeCredential = if (provider == SyncProviderModel.Gist) removeToken else removePassword
    val hasCredential = if (provider == SyncProviderModel.Gist) sync.config.hasGithubToken else sync.config.hasWebdavPassword
    val draft = SyncConnectionDraft(provider, gistId, url, username, credential, removeCredential)
    val issue = draft.firstIssue(sync.config).takeIf { attempted }
    val dirty = providerName != initialProvider || gistId != original.gistId || url != original.webdavUrl ||
        username != original.webdavUsername || token.isNotEmpty() || password.isNotEmpty() || removeToken || removePassword
    val scope = rememberCoroutineScope()
    val list = rememberLazyListState()
    val addressFocus = remember { FocusRequester() }
    val credentialFocus = remember { FocusRequester() }
    val focus = LocalFocusManager.current
    val blocked = saving || !sync.loaded || sync.phase.isRunning

    fun close() { if (!saving) { if (dirty) discard = true else onDismiss() } }
    fun save() {
        if (saving || !repository.sync.value.loaded || repository.sync.value.phase.isRunning) return
        attempted = true
        val firstIssue = draft.firstIssue(sync.config)
        if (firstIssue != null) {
            scope.launch {
                list.scrollToItem(3)
                withFrameNanos { }
                if (firstIssue == SyncSetupIssue.Credential) credentialFocus.requestFocus() else addressFocus.requestFocus()
            }
            return
        }
        focus.clearFocus()
        saving = true
        error = null
        val changed = draft.destinationChanged(sync.config) || removeCredential
        scope.launch {
            try {
                val result = withContext(Dispatchers.IO) { saveSyncConnection(repository, draft) }
                result.fold(onSuccess = { token = ""; password = ""; onSaved(changed) }, onFailure = { failure ->
                    error = if (failure is SyncSetupSaveException && failure.credentialSaved) {
                        uiString(R.string.sync_mobile_partial_save, describeSyncError(failure.cause ?: failure))
                    } else describeSyncError(failure.cause ?: failure)
                    list.scrollToItem(0)
                })
            } finally { saving = false }
        }
    }

    Dialog(onDismissRequest = ::close, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        AdaptiveWorkspace {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) {
                Scaffold(
                    modifier = Modifier.widthIn(max = 720.dp).fillMaxSize().testTag("sync-connection-editor"),
                    containerColor = MaterialTheme.colorScheme.surfaceContainer,
                    topBar = { TopAppBar(title = { Text(stringResource(R.string.sync_mobile_destination), style = MaterialTheme.typography.headlineSmall) },
                        navigationIcon = { IconButton(onClick = ::close, enabled = !saving, modifier = Modifier.testTag("sync-editor-back")) {
                            Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.action_back))
                        } }) },
                    bottomBar = {
                        Surface(color = MaterialTheme.colorScheme.surfaceContainer) {
                            Button(onClick = ::save, enabled = !blocked,
                                modifier = Modifier.navigationBarsPadding().imePadding().padding(UiSpacing.sm).fillMaxWidth().testTag("sync-save-connection")) {
                                if (saving) { CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp); Spacer(Modifier.width(UiSpacing.xs)) }
                                Text(stringResource(if (saving) R.string.sync_mobile_saving else R.string.sync_mobile_save_connection))
                            }
                        }
                    }
                ) { padding ->
                    LazyColumn(state = list, modifier = Modifier.fillMaxSize().padding(padding).consumeWindowInsets(padding),
                        contentPadding = PaddingValues(UiSpacing.sm), verticalArrangement = Arrangement.spacedBy(UiSpacing.sm)) {
                        item {
                            Column(verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                                Text(stringResource(R.string.sync_mobile_connection_intro), style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant)
                                error?.let { SyncMessage(it) }
                            }
                        }
                        item {
                            Column(Modifier.selectableGroup(), verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                                for (option in listOf(SyncProviderModel.Gist, SyncProviderModel.WebDav)) {
                                    val selected = provider == option
                                    Surface(color = if (selected) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surfaceContainerLowest,
                                        contentColor = if (selected) MaterialTheme.colorScheme.onSecondaryContainer else MaterialTheme.colorScheme.onSurface,
                                        shape = MaterialTheme.shapes.large) {
                                        Row(Modifier.fillMaxWidth().selectable(selected, enabled = !blocked, role = Role.RadioButton,
                                            onClick = { providerName = option.name; attempted = false; error = null }).padding(UiSpacing.sm),
                                            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(UiSpacing.sm)) {
                                            RadioButton(selected, onClick = null, enabled = !blocked)
                                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
                                                Text(providerLabel(option), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                                                Text(stringResource(if (option == SyncProviderModel.Gist) R.string.sync_mobile_gist_description else R.string.sync_mobile_webdav_description),
                                                    style = MaterialTheme.typography.bodyMedium)
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        item { Text(stringResource(R.string.sync_mobile_destination_fields), style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.semantics { heading() }) }
                        // Address and credential fields always occupy this item, enabling predictable first-error focus.
                        item {
                            Column(verticalArrangement = Arrangement.spacedBy(UiSpacing.sm)) {
                                if (provider == SyncProviderModel.Gist) {
                                    OutlinedTextField(gistId, { gistId = it }, enabled = !blocked, singleLine = true,
                                        modifier = Modifier.fillMaxWidth().focusRequester(addressFocus).testTag("sync-gist-id"),
                                        label = { Text(stringResource(R.string.desktop_sync_gist_id)) },
                                        supportingText = { Text(stringResource(if (issue == SyncSetupIssue.GistId) R.string.sync_mobile_invalid_gist else R.string.sync_mobile_gist_id_hint)) },
                                        isError = issue == SyncSetupIssue.GistId, keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next),
                                        keyboardActions = KeyboardActions(onNext = { credentialFocus.requestFocus() }))
                                } else {
                                    OutlinedTextField(url, { url = it }, enabled = !blocked, singleLine = true,
                                        modifier = Modifier.fillMaxWidth().focusRequester(addressFocus).testTag("sync-webdav-url"),
                                        label = { Text(stringResource(R.string.sync_mobile_webdav_file_url)) },
                                        supportingText = { Text(stringResource(if (issue == SyncSetupIssue.WebDavUrl) R.string.sync_mobile_invalid_url else R.string.sync_mobile_url_hint)) },
                                        isError = issue == SyncSetupIssue.WebDavUrl,
                                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next))
                                    OutlinedTextField(username, { username = it }, enabled = !blocked, singleLine = true,
                                        modifier = Modifier.fillMaxWidth(), label = { Text(stringResource(R.string.settings_sync_username)) },
                                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next))
                                }
                                SyncSecretField(credential, { if (provider == SyncProviderModel.Gist) { token = it; removeToken = false } else { password = it; removePassword = false } },
                                    stringResource(if (provider == SyncProviderModel.Gist) R.string.desktop_sync_gist_token else R.string.desktop_sync_webdav_password),
                                    enabled = !blocked, modifier = Modifier.focusRequester(credentialFocus).testTag("sync-credential"),
                                    error = issue == SyncSetupIssue.Credential,
                                    supporting = stringResource(when {
                                        issue == SyncSetupIssue.Credential -> R.string.sync_mobile_credential_required
                                        removeCredential -> R.string.sync_mobile_credential_will_remove
                                        hasCredential -> R.string.sync_mobile_credential_keep
                                        provider == SyncProviderModel.Gist -> R.string.sync_mobile_token_hint
                                        else -> R.string.sync_mobile_password_hint
                                    }), onDone = ::save)
                                if (hasCredential) TextButton(enabled = !blocked, onClick = {
                                    if (removeCredential) { if (provider == SyncProviderModel.Gist) removeToken = false else removePassword = false }
                                    else removeConfirm = true
                                }) { Text(stringResource(if (removeCredential) R.string.sync_mobile_keep_credential else R.string.sync_mobile_remove_credential)) }
                            }
                        }
                        if (draft.destinationChanged(original) && original.provider != SyncProviderModel.None) item {
                            Text(stringResource(R.string.sync_mobile_destination_change_hint), style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        if (sync.vaultLocked) item { SyncMessage(uiString(R.string.ui_vault_locked_unlock_it_to_resume_sync)) }
                    }
                }
            }
        }
    }
    if (discard) SyncConfirmation(stringResource(R.string.sync_mobile_discard_title), stringResource(R.string.sync_mobile_discard_body),
        onDismiss = { discard = false }, onConfirm = onDismiss)
    if (removeConfirm) SyncConfirmation(stringResource(R.string.sync_mobile_remove_credential), stringResource(R.string.sync_mobile_remove_credential_warning),
        onDismiss = { removeConfirm = false }, onConfirm = {
            removeConfirm = false
            if (provider == SyncProviderModel.Gist) { removeToken = true; token = "" } else { removePassword = true; password = "" }
        })
}

@Composable
private fun SyncSecretField(value: String, onChange: (String) -> Unit, label: String, enabled: Boolean,
    modifier: Modifier = Modifier, error: Boolean = false, supporting: String? = null, imeAction: ImeAction = ImeAction.Done, onDone: () -> Unit) {
    var visible by remember { mutableStateOf(false) }
    OutlinedTextField(value, onChange, enabled = enabled, singleLine = true, isError = error,
        modifier = modifier.fillMaxWidth(), label = { Text(label) },
        supportingText = supporting?.let { { Text(it) } },
        visualTransformation = if (visible) VisualTransformation.None else PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = imeAction),
        keyboardActions = KeyboardActions(onNext = { onDone() }, onDone = { onDone() }),
        trailingIcon = { IconButton(enabled = enabled, onClick = { visible = !visible }) {
            Icon(if (visible) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                stringResource(if (visible) R.string.sync_mobile_hide_secret else R.string.sync_mobile_show_secret))
        } })
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SyncPassphraseEditor(onDismiss: () -> Unit, onSaved: () -> Unit) {
    val repository = LocalMiaominalRepository.current
    val sync by repository.sync.collectAsState()
    var value by remember { mutableStateOf("") }
    var confirmation by remember { mutableStateOf("") }
    var attempted by remember { mutableStateOf(false) }
    var saving by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var clearConfirm by remember { mutableStateOf(false) }
    var discard by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val issue = syncPassphraseIssue(value, confirmation).takeIf { attempted }
    val firstFocus = remember { FocusRequester() }
    val confirmFocus = remember { FocusRequester() }
    val focus = LocalFocusManager.current
    val blocked = saving || !sync.loaded || sync.phase.isRunning
    fun close() { if (!saving) { if (value.isNotEmpty() || confirmation.isNotEmpty()) discard = true else onDismiss() } }
    fun save(clear: Boolean = false) {
        if (saving || !repository.sync.value.loaded || repository.sync.value.phase.isRunning) return
        if (!clear) {
            attempted = true
            val firstIssue = syncPassphraseIssue(value, confirmation)
            if (firstIssue != null) {
                if (firstIssue == SyncPassphraseIssue.Empty) firstFocus.requestFocus() else confirmFocus.requestFocus()
                return
            }
        }
        saving = true
        error = null
        focus.clearFocus()
        scope.launch {
            try {
                val result = syncRequest {
                    if (clear && sync.config.autoSyncEnabled) repository.syncSetAutoSync(false).getOrThrow()
                    repository.setSyncCredential(SyncCredentialKindModel.Passphrase,
                        if (clear) CredentialUpdate.Clear else CredentialUpdate.Set(value))
                }
                result.fold(onSuccess = { value = ""; confirmation = ""; onSaved() }, onFailure = { error = describeSyncError(it) })
            } finally { saving = false }
        }
    }
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true, confirmValueChange = { target ->
        if (target == SheetValue.Hidden && (saving || value.isNotEmpty() || confirmation.isNotEmpty())) {
            if (!saving) discard = true
            false
        } else true
    })
    ModalBottomSheet(onDismissRequest = ::close, sheetState = sheet,
        sheetGesturesEnabled = !saving) {
        Column(Modifier.fillMaxWidth().imePadding().verticalScroll(androidx.compose.foundation.rememberScrollState()).padding(UiSpacing.sm),
            verticalArrangement = Arrangement.spacedBy(UiSpacing.sm)) {
            Text(stringResource(R.string.desktop_sync_passphrase), style = MaterialTheme.typography.headlineSmall,
                modifier = Modifier.semantics { heading() })
            Text(stringResource(R.string.sync_mobile_passphrase_hint), style = MaterialTheme.typography.bodyMedium)
            if (sync.config.hasPassphrase) Text(stringResource(R.string.sync_mobile_passphrase_change_hint),
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            SyncSecretField(value, { value = it }, stringResource(R.string.desktop_sync_passphrase), !blocked,
                modifier = Modifier.focusRequester(firstFocus).testTag("sync-passphrase"), error = issue == SyncPassphraseIssue.Empty,
                supporting = if (issue == SyncPassphraseIssue.Empty) stringResource(R.string.sync_mobile_passphrase_required) else null,
                imeAction = ImeAction.Next, onDone = { confirmFocus.requestFocus() })
            SyncSecretField(confirmation, { confirmation = it }, stringResource(R.string.sync_mobile_confirm_passphrase), !blocked,
                modifier = Modifier.focusRequester(confirmFocus).testTag("sync-passphrase-confirm"), error = issue == SyncPassphraseIssue.Mismatch,
                supporting = if (issue == SyncPassphraseIssue.Mismatch) stringResource(R.string.sync_mobile_passphrase_mismatch) else null,
                onDone = { save() })
            error?.let { SyncMessage(it) }
            Button(onClick = { save() }, enabled = !blocked, modifier = Modifier.fillMaxWidth().testTag("sync-save-passphrase")) {
                if (saving) { CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp); Spacer(Modifier.width(UiSpacing.xs)) }
                Text(stringResource(if (saving) R.string.sync_mobile_saving else R.string.settings_action_save))
            }
            if (sync.config.hasPassphrase) TextButton(onClick = { clearConfirm = true }, enabled = !blocked, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.desktop_sync_passphrase_clear), color = MaterialTheme.colorScheme.error)
            }
            TextButton(onClick = ::close, enabled = !saving, modifier = Modifier.fillMaxWidth()) { Text(stringResource(R.string.action_cancel)) }
        }
    }
    if (discard) SyncConfirmation(stringResource(R.string.sync_mobile_discard_title), stringResource(R.string.sync_mobile_discard_body),
        onDismiss = { discard = false }, onConfirm = onDismiss)
    if (clearConfirm) SyncConfirmation(stringResource(R.string.desktop_sync_passphrase_clear), stringResource(R.string.sync_mobile_clear_passphrase_warning),
        onDismiss = { clearConfirm = false }, onConfirm = { clearConfirm = false; save(clear = true) })
}

private suspend fun syncRequest(action: suspend () -> Result<Unit>): Result<Unit> = withContext(Dispatchers.IO) {
    try { action() } catch (error: CancellationException) { throw error }
    catch (error: Throwable) { Result.failure(error) }
}
