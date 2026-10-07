package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.ui.theme.UiSpacing
import dev.akko.miaominal.ui.theme.LocalUiPreferences
import dev.akko.miaominal.ui.theme.UiContrast

import dev.akko.miaominal.ui.uiString

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.StringRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Remove
import androidx.compose.material.icons.outlined.Sync
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import kotlinx.coroutines.flow.first
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import dev.akko.miaominal.MiaominalApplication
import dev.akko.miaominal.R
import dev.akko.miaominal.data.ImportSourceModel
import dev.akko.miaominal.data.NativeBackendClient
import dev.akko.miaominal.data.NativeBackendState
import dev.akko.miaominal.data.SyncPhaseModel
import dev.akko.miaominal.nativebridge.BackendException
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.BrandMark
import dev.akko.miaominal.ui.components.PageSubtitle
import dev.akko.miaominal.ui.components.SectionTitle
import dev.akko.miaominal.ui.components.SeedColorPicker
import dev.akko.miaominal.ui.model.appVersionName
import dev.akko.miaominal.ui.theme.ThemeMode
import kotlinx.coroutines.launch
import java.text.DateFormat
import java.util.Date

/** Taps on the version badge that reveal the developer diagnostics section. */
private const val DIAGNOSTIC_TAP_COUNT = 7

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsDetailScreen(
    destination: SettingsDestination,
    themeMode: ThemeMode,
    dynamicColor: Boolean,
    seedColor: Int,
    onThemeModeChange: (ThemeMode) -> Unit,
    onDynamicColorChange: (Boolean) -> Unit,
    onSeedColorChange: (Int) -> Unit,
    onReopenOnboarding: () -> Unit,
    onDismiss: () -> Unit,
    onManageProxies: () -> Unit = {},
    /** Section to scroll to and highlight after arriving from a search result. */
    focusAnchor: String? = null,
    modifier: Modifier = Modifier
) {
    val operations: dev.akko.miaominal.ui.model.SettingsOperations = androidx.lifecycle.viewmodel.compose.viewModel(key = "settings-operations")
    val effectiveAnchor = dev.akko.miaominal.ui.model.terminalSettingsAnchor(focusAnchor)
    val effectiveDestination = if (focusAnchor == "appearance-typography") SettingsDestination.TerminalKeys else destination
    Scaffold(
        modifier = modifier,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(effectiveDestination.titleRes), style = MaterialTheme.typography.headlineSmall) },
                navigationIcon = {
                    IconButton(onClick = onDismiss) {
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
            .wrapContentWidth(Alignment.CenterHorizontally)
            .widthIn(max = 720.dp)
            .fillMaxWidth()

        when (effectiveDestination) {
            SettingsDestination.Appearance -> AppearanceSettingsPage(
                themeMode = themeMode,
                dynamicColor = dynamicColor,
                seedColor = seedColor,
                onThemeModeChange = onThemeModeChange,
                onDynamicColorChange = onDynamicColorChange,
                onSeedColorChange = onSeedColorChange,
                focusAnchor = focusAnchor,
                modifier = contentModifier
            )
            SettingsDestination.Connections -> ConnectionsSettingsPage(contentModifier, focusAnchor, operations, onManageProxies)
            SettingsDestination.TerminalKeys ->
                MobileTerminalSettingsPage(contentModifier, effectiveAnchor)
            SettingsDestination.Vault -> MobileVaultSettingsPage(operations, contentModifier, focusAnchor)
            SettingsDestination.Sync -> SyncSettingsPage(contentModifier, focusAnchor)
            SettingsDestination.Ai -> AgentConfigSettingsPage(contentModifier)
            SettingsDestination.About -> AboutSettingsPage(
                onReopenOnboarding = onReopenOnboarding,
                focusAnchor = focusAnchor,
                modifier = contentModifier
            )
        }
    }
}

/**
 * Scrolls a settings page to the section a search result points at.
 *
 * Items are found by their LazyColumn key: the list is walked from the top until
 * the section is composed, so the jump stays correct when a page gains or loses
 * conditional items.
 */
@Composable
internal fun SettingsAnchorScroll(state: LazyListState, anchor: String?) {
    LaunchedEffect(anchor) {
        if (anchor == null) return@LaunchedEffect
        state.scrollToItem(0)
        var guard = 0
        while (guard++ < 120) {
            val info = state.layoutInfo
            if (info.visibleItemsInfo.isEmpty()) {
                withFrameNanos { }
                continue
            }
            val match = info.visibleItemsInfo.firstOrNull { it.key == anchor }
            if (match != null) {
                state.animateScrollToItem(match.index)
                return@LaunchedEffect
            }
            val last = info.visibleItemsInfo.lastOrNull() ?: return@LaunchedEffect
            if (last.index >= info.totalItemsCount - 1) break
            val next = (last.index + 1).coerceAtMost(info.totalItemsCount - 1)
            state.scrollToItem(next)
            // The section only exists once it is composed, so wait for the layout
            // to reflect the jump before looking for the key again.
            snapshotFlow { state.layoutInfo.visibleItemsInfo.lastOrNull()?.index }
                .first { it != null && it >= next }
        }
        // Sections can be conditional (a pending sync conflict, for example): when
        // the anchor is not on screen at all, leave the page at its top.
        state.animateScrollToItem(0)
    }
}

private enum class AppLanguageOption(@param:StringRes val labelRes: Int) {
    English(R.string.language_english),
    SimplifiedChinese(R.string.language_chinese)
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AppearanceSettingsPage(
    themeMode: ThemeMode,
    dynamicColor: Boolean,
    seedColor: Int,
    onThemeModeChange: (ThemeMode) -> Unit,
    onDynamicColorChange: (Boolean) -> Unit,
    onSeedColorChange: (Int) -> Unit,
    focusAnchor: String? = null,
    modifier: Modifier = Modifier
) {
    val repository = LocalMiaominalRepository.current
    val scope = rememberCoroutineScope()
    val preferences by repository.settings.collectAsState()
    val language = AppLanguageOption.entries.firstOrNull { it.name == preferences.language }
        ?: AppLanguageOption.SimplifiedChinese
    val uiPreferences = LocalUiPreferences.current
    val appearance = dev.akko.miaominal.ui.model.LocalAppearanceOperations.current
    var appearanceError by rememberSaveable { mutableStateOf<String?>(null) }
        val listState = rememberLazyListState()
    SettingsAnchorScroll(listState, focusAnchor)
    LazyColumn(state = listState,
        modifier = modifier,
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(UiSpacing.sm)
    ) {
        item { PageSubtitle(text = stringResource(R.string.desktop_appearance_subtitle)) }
        item {
            if (appearance?.busy == true || uiPreferences?.busy == true) LinearSettingsProgress()
            appearance?.error?.let { SettingsErrorText(stringResource(R.string.settings_save_failed) + "\n" + describeSyncError(it)) }
            if (uiPreferences?.saveFailed == true) SettingsErrorText(stringResource(R.string.settings_save_failed))
        }
        item(key = "appearance-theme") { SectionTitle(title = stringResource(R.string.appearance_theme_group), focused = focusAnchor == "appearance-theme") }
        item {
            SettingsDetailCard {
                Text(
                    text = stringResource(R.string.appearance_theme_mode),
                    style = MaterialTheme.typography.titleSmall
                )
                SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                    ThemeMode.entries.forEachIndexed { index, mode ->
                        val label = when (mode) {
                            ThemeMode.System -> stringResource(R.string.appearance_theme_system)
                            ThemeMode.Light -> stringResource(R.string.appearance_theme_light)
                            ThemeMode.Dark -> stringResource(R.string.appearance_theme_dark)
                        }
                        SegmentedButton(
                            selected = themeMode == mode,
                            enabled = appearance?.busy != true,
                            onClick = { onThemeModeChange(mode) },
                            shape = SegmentedButtonDefaults.itemShape(
                                index = index,
                                count = ThemeMode.entries.size
                            )
                        ) {
                            Text(label)
                        }
                    }
                }
                SettingsToggleRow(
                    title = stringResource(R.string.appearance_dynamic_color),
                    subtitle = stringResource(R.string.appearance_dynamic_color_desc),
                    checked = dynamicColor,
                    enabled = appearance?.busy != true,
                    onCheckedChange = onDynamicColorChange
                )
                if (!dynamicColor) {
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                    SeedColorPicker(
                        seedColor = seedColor,
                        onSeedColorChange = onSeedColorChange
                    )
                }
            }
        }
        item { uiPreferences?.let { ui ->
                    Text(stringResource(R.string.ux_contrast), style = MaterialTheme.typography.titleSmall)
                    androidx.compose.foundation.layout.FlowRow(horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                        UiContrast.entries.forEach { option -> FilterChip(selected = ui.contrast == option, enabled = !ui.busy,
                            onClick = { ui.updateContrast(option) }, label = { Text(stringResource(when (option) {
                                UiContrast.System -> R.string.ux_contrast_system
                                UiContrast.Standard -> R.string.ux_contrast_standard
                                UiContrast.Medium -> R.string.ux_contrast_medium
                                UiContrast.High -> R.string.ux_contrast_high
                            })) }) }
                    }
                }
        }
        item(key = "appearance-language") { SectionTitle(title = stringResource(R.string.desktop_appearance_language_group), focused = focusAnchor == "appearance-language") }
        item {
            SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                AppLanguageOption.entries.forEachIndexed { index, option ->
                    SegmentedButton(
                        selected = language == option,
                        onClick = {
                            scope.launch {
                                repository.updateSettings { it.copy(language = option.name) }.onFailure { appearanceError = describeSyncError(it) }
                            }
                        },
                        shape = SegmentedButtonDefaults.itemShape(
                            index = index,
                            count = AppLanguageOption.entries.size
                        )
                    ) {
                        Text(stringResource(option.labelRes))
                    }
                }
            }
        }
        item { appearanceError?.let { SettingsErrorText(it) } }
    }
}

private enum class MonitorHistory(@param:StringRes val labelRes: Int) {
    OneMinute(R.string.desktop_connections_history_1),
    FiveMinutes(R.string.desktop_connections_history_5),
    TenMinutes(R.string.desktop_connections_history_10),
    ThirtyMinutes(R.string.desktop_connections_history_30)
}

private enum class LastSessionCloseBehavior(@param:StringRes val labelRes: Int) {
    ExitApplication(R.string.desktop_connections_close_exit),
    OpenHosts(R.string.desktop_connections_close_hosts)
}

private enum class ImportSource(@param:StringRes val labelRes: Int) {
    OpenSsh(R.string.desktop_connections_import_openssh),
    Putty(R.string.desktop_connections_import_putty),
    SecureCrt(R.string.desktop_connections_import_securecrt),
    FinalShell(R.string.desktop_connections_import_finalshell);

    fun toModel(): ImportSourceModel = when (this) {
        OpenSsh -> ImportSourceModel.OpenSsh
        Putty -> ImportSourceModel.Putty
        SecureCrt -> ImportSourceModel.SecureCrt
        FinalShell -> ImportSourceModel.FinalShell
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ConnectionsSettingsPage(modifier: Modifier = Modifier, focusAnchor: String? = null, operations: dev.akko.miaominal.ui.model.SettingsOperations, onManageProxies: () -> Unit) {
    val repository = LocalMiaominalRepository.current
    val scope = rememberCoroutineScope()
    val business by repository.settings.collectAsState()
    val preferences by repository.devicePreferences.collectAsState()
    val recentCount = business.recentHostCount
    val autoMonitoring = business.autoMonitoring
    val monitorHistory = MonitorHistory.entries
        .firstOrNull { it.name == business.monitorHistory }
        ?: MonitorHistory.FiveMinutes
    val closeBehavior = LastSessionCloseBehavior.entries
        .firstOrNull { it.name == business.closeLastSessionBehavior }
        ?: LastSessionCloseBehavior.OpenHosts
    val importSource = ImportSource.entries
        .firstOrNull { it.name == preferences.importSource }
        ?: ImportSource.OpenSsh
    val proxyAddress = preferences.proxyAddress
    val proxyPort = preferences.proxyPort
    val context = LocalContext.current
    val backgroundPolicy = remember(context) {
        (context.applicationContext as? MiaominalApplication)?.container?.backgroundPolicy
    }
    var notificationsAllowed by remember { mutableStateOf(androidx.core.app.NotificationManagerCompat.from(context).areNotificationsEnabled()) }
    androidx.lifecycle.compose.LifecycleResumeEffect(Unit) {
        notificationsAllowed = androidx.core.app.NotificationManagerCompat.from(context).areNotificationsEnabled()
        onPauseOrDispose { }
    }
    val notificationPermissionLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestPermission()
    ) { granted ->
        notificationsAllowed = androidx.core.app.NotificationManagerCompat.from(context).areNotificationsEnabled()
        backgroundPolicy?.setKeepAliveEnabled(true)
        if (!granted) {
            // The service still runs; the notification is just hidden by the OS.
            android.util.Log.w("BackgroundPolicy", "POST_NOTIFICATIONS was not granted")
        }
    }
        val listState = rememberLazyListState()
    SettingsAnchorScroll(listState, focusAnchor)
    LazyColumn(state = listState,
        modifier = modifier.testTag("connections-settings-list"),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(UiSpacing.sm)
    ) {
        item { PageSubtitle(text = stringResource(R.string.desktop_connections_subtitle)) }
        item(key = "connections-recent") { SectionTitle(title = stringResource(R.string.desktop_connections_recent_group), focused = focusAnchor == "connections-recent") }
        item {
            SettingsDetailCard {
                Text(
                    text = stringResource(R.string.desktop_connections_recent_desc),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = stringResource(R.string.desktop_connections_recent_count),
                        modifier = Modifier.weight(1f),
                        style = MaterialTheme.typography.titleSmall
                    )
                    IconButton(
                        enabled = recentCount > 0,
                        onClick = {
                            scope.launch {
                                repository.updateSettings {
                                    it.copy(recentHostCount = (it.recentHostCount - 1).coerceAtLeast(0))
                                }
                            }
                        }
                    ) {
                        Icon(Icons.Outlined.Remove, contentDescription = stringResource(R.string.font_decrease, stringResource(R.string.desktop_connections_recent_count)))
                    }
                    Text(
                        text = recentCount.toString(),
                        style = MaterialTheme.typography.titleMedium
                    )
                    IconButton(
                        enabled = recentCount < 20,
                        onClick = {
                            scope.launch {
                                repository.updateSettings {
                                    it.copy(recentHostCount = (it.recentHostCount + 1).coerceAtMost(20))
                                }
                            }
                        }
                    ) {
                        Icon(Icons.Outlined.Add, contentDescription = stringResource(R.string.font_increase, stringResource(R.string.desktop_connections_recent_count)))
                    }
                }
            }
        }
        item(key = "connections-monitor") { SectionTitle(title = stringResource(R.string.desktop_connections_monitor_group), focused = focusAnchor == "connections-monitor") }
        item {
            SettingsDetailCard {
                SettingsToggleRow(
                    title = stringResource(R.string.desktop_connections_auto_monitor),
                    subtitle = stringResource(R.string.desktop_connections_auto_monitor_desc),
                    checked = autoMonitoring,
                    onCheckedChange = { checked ->
                        scope.launch {
                            repository.updateSettings { it.copy(autoMonitoring = checked) }
                        }
                    }
                )
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                Text(
                    text = stringResource(R.string.desktop_connections_monitor_history),
                    style = MaterialTheme.typography.titleSmall
                )
                FlowRow(horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
                    MonitorHistory.entries.forEach { option ->
                        FilterChip(
                            selected = monitorHistory == option,
                            onClick = {
                                scope.launch {
                                    repository.updateSettings { it.copy(monitorHistory = option.name) }
                                }
                            },
                            label = { Text(stringResource(option.labelRes)) }
                        )
                    }
                }
            }
        }
        item(key = "connections-session") { SectionTitle(title = stringResource(R.string.desktop_connections_session_group), focused = focusAnchor == "connections-session") }
        item {
            SettingsDetailCard {
                SettingsToggleRow(
                    title = stringResource(R.string.background_keep_alive),
                    subtitle = stringResource(R.string.background_keep_alive_desc),
                    checked = preferences.backgroundKeepAlive,
                    onCheckedChange = { checked ->
                        if (checked) {
                            val needsPermission = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                                ContextCompat.checkSelfPermission(
                                    context,
                                    Manifest.permission.POST_NOTIFICATIONS
                                ) != PackageManager.PERMISSION_GRANTED
                            scope.launch {
                                repository.updateDevicePreferences {
                                    it.copy(backgroundKeepAlive = true)
                                }
                                if (needsPermission) {
                                    notificationPermissionLauncher.launch(
                                        Manifest.permission.POST_NOTIFICATIONS
                                    )
                                } else {
                                    backgroundPolicy?.setKeepAliveEnabled(true)
                                }
                            }
                        } else {
                            scope.launch {
                                repository.updateDevicePreferences {
                                    it.copy(backgroundKeepAlive = false)
                                }
                                backgroundPolicy?.setKeepAliveEnabled(false)
                            }
                        }
                    }
                )
            }
        }
        item {
            SettingsDetailCard {
                Text(stringResource(if (notificationsAllowed) R.string.settings_notifications_allowed else R.string.settings_notifications_denied))
                if (!notificationsAllowed) TextButton(onClick = {
                    context.startActivity(android.content.Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                        .putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, context.packageName))
                }) { Text(stringResource(R.string.settings_system_settings)) }
            }
        }
        item {
            SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                LastSessionCloseBehavior.entries.forEachIndexed { index, option ->
                    SegmentedButton(
                        selected = closeBehavior == option,
                        onClick = {
                            scope.launch {
                                repository.updateSettings {
                                    it.copy(closeLastSessionBehavior = option.name)
                                }
                            }
                        },
                        shape = SegmentedButtonDefaults.itemShape(
                            index = index,
                            count = LastSessionCloseBehavior.entries.size
                        )
                    ) {
                        Text(stringResource(option.labelRes))
                    }
                }
            }
        }
        item(key = "connections-proxy") { SectionTitle(title = stringResource(R.string.desktop_connections_proxy_group), focused = focusAnchor == "connections-proxy") }
        item {
            SettingsDetailCard {
                Text(stringResource(R.string.settings_proxy_hint))
                Button(onClick = onManageProxies) { Text(stringResource(R.string.settings_manage_proxies)) }
                if (proxyAddress.isNotBlank()) {
                    Text(stringResource(R.string.settings_legacy_proxy))
                    Text("$proxyAddress:$proxyPort", style = MaterialTheme.typography.bodyMedium)
                    TextButton(onClick = { copySettingsText(context, "$proxyAddress:$proxyPort") }) { Text(stringResource(R.string.settings_copy)) }
                }
            }
        }
        item(key = "connections-import") { SectionTitle(title = stringResource(R.string.desktop_connections_import_group), focused = focusAnchor == "connections-import") }
        item {
            SettingsDetailCard {
                Text(stringResource(R.string.settings_import_steps))
                Button(onClick = { operations.openImport(importSource.toModel()) }) { Text(stringResource(R.string.desktop_connections_import_action)) }
            }
        }
    }
    SettingsImportFlow(operations)
}

/** Typed sync failure text; category comes from Rust, never from string parsing. */
internal fun describeSyncError(error: Throwable): String {
    val message = error.message?.takeIf(String::isNotBlank)
    return when (error) {
        is BackendException.VaultLocked -> uiString(R.string.ui_vault_locked_unlock_it_to_resume_sync)
        is BackendException.SyncAuthentication -> uiString(R.string.ui_incorrect_sync_passphrase_or_authentication_rejected)
        is BackendException.SyncCapability -> uiString(R.string.ui_remote_storage_does_not_support_safe_conditional_writes_automatic)
        is BackendException.SyncNotConfigured -> uiString(R.string.ui_sync_setup_is_incomplete_credentials_or_encryption_passphrase_mis)
        is BackendException.SyncConflict -> uiString(R.string.ui_local_and_remote_changes_conflict_choose_which_version_to_keep)
        is BackendException.Network -> uiString(R.string.ui_network_error, message ?: uiString(R.string.ui_could_not_connect_to_the_sync_service))
        is BackendException.Cancelled -> uiString(R.string.ui_operation_cancelled)
        is BackendException.Conflict -> uiString(R.string.ui_remote_content_changed_nothing_was_overwritten)
        is BackendException.Storage -> uiString(R.string.ui_sync_storage_error, message ?: uiString(R.string.ui_unknown_error))
        is BackendException.InvalidArgument -> message ?: uiString(R.string.ui_invalid_parameter)
        else -> message ?: error.javaClass.simpleName
    }
}

internal fun syncPhaseLabel(phase: SyncPhaseModel): String = when (phase) {
    SyncPhaseModel.Disabled -> uiString(R.string.vault_summary_disabled)
    SyncPhaseModel.Idle -> uiString(R.string.desktop_sync_idle)
    SyncPhaseModel.CheckingCapability -> uiString(R.string.ui_checking_remote_capabilities)
    SyncPhaseModel.CheckingRemote -> uiString(R.string.ui_checking_remote)
    SyncPhaseModel.Pulling -> uiString(R.string.ui_pulling)
    SyncPhaseModel.Pushing -> uiString(R.string.ui_pushing)
    SyncPhaseModel.PausedVaultLocked -> uiString(R.string.ui_paused_vault_locked)
    SyncPhaseModel.PausedCredentials -> uiString(R.string.ui_paused_credentials_missing)
    SyncPhaseModel.PausedCapability -> uiString(R.string.ui_paused_remote_capabilities_insufficient)
    SyncPhaseModel.Conflict -> uiString(R.string.ui_conflict_requires_a_decision)
    SyncPhaseModel.Cancelling -> uiString(R.string.ui_cancelling)
}

internal fun syncInterventionLabel(reason: String): String = when (reason) {
    "BOTH_SIDES_CHANGED" -> uiString(R.string.ui_both_local_and_remote_content_changed)
    "REMOTE_CHANGED_BEFORE_PUSH" -> uiString(R.string.ui_remote_content_changed_before_pushing)
    "UNSAFE_PROVIDER_WRITE" -> uiString(R.string.ui_remote_storage_cannot_guarantee_safe_writes)
    "LOCAL_CHANGED_DURING_PULL" -> uiString(R.string.ui_local_content_changed_during_pull)
    "MISSING_SYNC_BASELINE" -> uiString(R.string.ui_no_sync_baseline_confirm_the_initial_pull)
    "SYNC_CONFIGURATION_CHANGED" -> uiString(R.string.ui_sync_configuration_changed)
    else -> reason
}

/** User-facing reason for a sync failure; never the internal enum name. */
internal fun syncFailureReason(kind: String): String = when (kind.uppercase()) {
    "AUTHENTICATION" -> uiString(R.string.ui_remote_credentials_are_invalid_or_expired)
    "NETWORK" -> uiString(R.string.ui_network_unavailable_or_remote_service_not_responding)
    "CAPABILITY" -> uiString(R.string.ui_remote_service_does_not_support_this_sync_method)
    "CONFLICT" -> uiString(R.string.ui_both_local_and_remote_content_changed)
    "LOCKED" -> uiString(R.string.ui_vault_locked)
    "NOTCONFIGURED" -> uiString(R.string.ui_incomplete_sync_configuration)
    "PROVIDER" -> uiString(R.string.ui_remote_service_rejected_the_operation)
    "STORAGE" -> uiString(R.string.ui_local_storage_unavailable)
    "CANCELLED" -> uiString(R.string.ui_operation_cancelled)
    else -> uiString(R.string.ui_sync_failed_102)
}

/** What the user can do about a failure; null when nothing is actionable. */
internal fun syncFailureAdvice(kind: String): String? = when (kind.uppercase()) {
    "AUTHENTICATION" -> uiString(R.string.ui_enter_the_token_or_password_again_and_retry)
    "NETWORK" -> uiString(R.string.ui_check_the_network_or_proxy_settings_and_retry)
    "CAPABILITY" -> uiString(R.string.ui_check_remote_capabilities_or_use_another_sync_method)
    "CONFLICT" -> uiString(R.string.ui_choose_which_version_to_keep_in_the_conflict_section_below)
    "LOCKED" -> uiString(R.string.ui_sync_resumes_after_you_unlock_the_vault)
    "NOTCONFIGURED" -> uiString(R.string.ui_complete_sync_setup_and_retry)
    "PROVIDER" -> uiString(R.string.ui_retry_later_or_check_remote_repository_permissions)
    "STORAGE" -> uiString(R.string.ui_free_up_device_storage_and_retry)
    else -> null
}

/** Local date and time for a remote timestamp (never raw milliseconds). */
internal fun formatSyncTimestamp(millis: Long): String =
    DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT)
        .format(Date(millis))

@Composable
private fun AboutSettingsPage(
    onReopenOnboarding: () -> Unit,
    focusAnchor: String? = null,
    modifier: Modifier = Modifier
) {
    val context = LocalContext.current
    // Developer diagnostics are not part of the product surface: seven taps on
    // the version badge reveal the native-backend card for this session.
    var diagnosticsVisible by rememberSaveable { mutableStateOf(false) }
    var versionTaps by rememberSaveable { mutableIntStateOf(0) }
    val versionName = remember(context) { appVersionName(context) }
    val backendClient = remember(context) { NativeBackendClient.get(context) }
    val backendState by backendClient.state.collectAsState()
    val backendVersion = (backendState as? NativeBackendState.Running)?.packageVersion
    val versionInfo = "Miaominal\n${stringResource(R.string.about_app_version)}: $versionName\n${stringResource(R.string.about_backend_version)}: ${backendVersion ?: stringResource(R.string.ui_unknown)}"

        val listState = rememberLazyListState()
    SettingsAnchorScroll(listState, focusAnchor)
    LazyColumn(state = listState,
        modifier = modifier,
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(UiSpacing.sm)
    ) {
        item { PageSubtitle(text = stringResource(R.string.desktop_about_subtitle)) }
        item {
            Card(
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.primaryContainer,
                    contentColor = MaterialTheme.colorScheme.onPrimaryContainer
                )
            ) {
                ListItem(
                    headlineContent = {
                        Text("Miaominal", style = MaterialTheme.typography.titleLarge)
                    },
                    supportingContent = {
                        Text(
                            stringResource(R.string.desktop_about_overview_desc),
                            style = MaterialTheme.typography.bodyMedium
                        )
                    },
                    leadingContent = { BrandMark() },
                    colors = ListItemDefaults.colors(
                        containerColor = Color.Transparent,
                        headlineColor = MaterialTheme.colorScheme.onPrimaryContainer,
                        supportingColor = MaterialTheme.colorScheme.onPrimaryContainer
                    )
                )
            }
        }
        item {
            SettingsDetailCard {
                SettingsInfoRow(
                    label = stringResource(R.string.about_app_version),
                    value = versionName,
                    monospace = true,
                    modifier = Modifier.defaultMinSize(minHeight = 48.dp).clickable {
                        versionTaps += 1
                        if (versionTaps >= DIAGNOSTIC_TAP_COUNT) diagnosticsVisible = true
                    }.testTag("about-app-version-badge")
                )
                SettingsInfoRow(
                    label = stringResource(R.string.about_backend_version),
                    value = backendVersion ?: stringResource(R.string.ui_unknown),
                    monospace = true
                )
                TextButton(onClick = { copySettingsText(context, versionInfo) }) { Text(stringResource(R.string.settings_copy_versions)) }
            }
        }
        item(key = "about-onboarding") { SectionTitle(title = stringResource(R.string.desktop_about_onboarding_group), focused = focusAnchor == "about-onboarding") }
        item {
            SettingsDetailCard {
                Text(
                    text = stringResource(R.string.desktop_about_onboarding_desc),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Button(onClick = onReopenOnboarding) {
                    Text(stringResource(R.string.desktop_about_onboarding_action))
                }
            }
        }
        if (diagnosticsVisible) {
            item(key = "about-diagnostics") { SectionTitle(title = uiString(R.string.ui_diagnostics), focused = focusAnchor == "about-diagnostics") }
            item { NativeBackendCard() }
        }
    }
}

@Composable
private fun NativeBackendCard() {
    val context = LocalContext.current
    val client = remember(context) { NativeBackendClient.get(context) }
    val state by client.state.collectAsState()
    val scope = rememberCoroutineScope()
    var restarting by remember { mutableStateOf(false) }

    SettingsDetailCard {
        when (val current = state) {
            NativeBackendState.Stopped -> {
                SettingsInfoRow(label = uiString(R.string.desktop_sync_status_group), value = uiString(R.string.ui_stopped))
            }

            is NativeBackendState.Starting -> {
                SettingsInfoRow(
                    label = uiString(R.string.desktop_sync_status_group),
                    value = uiString(R.string.ui_starting_generation, current.generation)
                )
            }

            is NativeBackendState.Running -> {
                SettingsInfoRow(
                    label = uiString(R.string.desktop_sync_status_group),
                    value = uiString(R.string.ui_running_generation, current.generation)
                )
                SettingsInfoRow(
                    label = uiString(R.string.ui_event_sequence),
                    value = current.sequence.toString(),
                    monospace = true
                )
                SettingsInfoRow(
                    label = uiString(R.string.ui_abi_version),
                    value = current.abiVersion,
                    monospace = true
                )
                SettingsInfoRow(
                    label = uiString(R.string.ui_device),
                    value = current.deviceLabel,
                    monospace = true
                )
                SettingsInfoRow(
                    label = uiString(R.string.ui_data_directory),
                    value = current.dataDir,
                    monospace = true
                )
                SettingsInfoRow(
                    label = uiString(R.string.ui_cache_directory),
                    value = current.cacheDir,
                    monospace = true
                )
                if (current.recentEvents.isNotEmpty()) {
                    Text(
                        text = uiString(R.string.ui_recent_events),
                        style = MaterialTheme.typography.titleSmall
                    )
                    current.recentEvents.forEach { event ->
                        Text(
                            text = event,
                            style = MaterialTheme.typography.bodySmall.copy(
                                fontFamily = FontFamily.Monospace
                            ),
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }

            is NativeBackendState.Failed -> {
                SettingsInfoRow(
                    label = uiString(R.string.desktop_sync_status_group),
                    value = uiString(R.string.ui_failed_generation, current.generation)
                )
                Text(
                    text = current.reason,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.error
                )
            }
        }
        OutlinedButton(
            enabled = !restarting,
            onClick = {
                restarting = true
                scope.launch {
                    try {
                        client.restart()
                    } finally {
                        restarting = false
                    }
                }
            }
        ) {
            if (restarting) {
                CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
            } else {
                Icon(Icons.Outlined.Refresh, contentDescription = null)
            }
            Spacer(Modifier.width(8.dp))
            Text(uiString(R.string.ui_restart_backend))
        }
    }
}

@Composable
internal fun SettingsDetailCard(content: @Composable ColumnScope.() -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
        )
    ) {
        Column(
            modifier = Modifier.padding(UiSpacing.sm),
            verticalArrangement = Arrangement.spacedBy(UiSpacing.sm),
            content = content
        )
    }
}

@Composable
internal fun SettingsToggleRow(
    title: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    subtitle: String? = null,
    enabled: Boolean = true
) {
    ListItem(
        modifier = Modifier
            .fillMaxWidth()
            .toggleable(value = checked, enabled = enabled, role = Role.Switch, onValueChange = onCheckedChange),
        headlineContent = {
            Text(title, style = MaterialTheme.typography.titleSmall)
        },
        supportingContent = subtitle?.let { supportingText ->
            {
                Text(
                    supportingText,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        },
        trailingContent = {
            Switch(checked = checked, onCheckedChange = null, enabled = enabled)
        },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent)
    )
}

@Composable
private fun SettingsInfoRow(
    label: String,
    value: String,
    monospace: Boolean = false,
    modifier: Modifier = Modifier
) {
    ListItem(
        modifier = modifier.fillMaxWidth(),
        headlineContent = {
            Text(
                text = label,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        },
        supportingContent = {
            Text(
                text = value,
                style = MaterialTheme.typography.bodyMedium.copy(
                    fontFamily = if (monospace) FontFamily.Monospace else FontFamily.Default
                )
            )
        },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent)
    )
}

internal fun copySettingsText(context: android.content.Context, value: String) {
    (context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager)
        .setPrimaryClip(android.content.ClipData.newPlainText("Miaominal", value))
}
