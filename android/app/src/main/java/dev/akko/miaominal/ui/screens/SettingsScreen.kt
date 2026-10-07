package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.ui.theme.UiSpacing
import androidx.annotation.StringRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Keyboard
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material.icons.outlined.Palette
import androidx.compose.material.icons.outlined.Security
import androidx.compose.material.icons.outlined.Sync
import androidx.compose.material3.Badge
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.vector.ImageVector

import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.ui.components.PageEmptyState
import dev.akko.miaominal.R
import dev.akko.miaominal.data.VaultMode
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.MiaominalSearchTopBar
import dev.akko.miaominal.ui.components.SectionTitle
import dev.akko.miaominal.ui.model.appVersionName

enum class SettingsDestination(
    @param:StringRes val titleRes: Int,
    /** Row subtitle; `null` means the row shows the app version instead. */
    @param:StringRes val descriptionRes: Int?,
    val icon: ImageVector
) {
    Appearance(R.string.setting_appearance, R.string.setting_appearance_desc, Icons.Outlined.Palette),
    Connections(R.string.setting_connections, R.string.setting_connections_desc, Icons.Outlined.Dns),
    TerminalKeys(
        R.string.setting_terminal_shortcuts,
        R.string.setting_terminal_shortcuts_desc,
        Icons.Outlined.Keyboard
    ),
    Vault(R.string.setting_vault, R.string.setting_vault_desc, Icons.Outlined.Security),
    Sync(R.string.setting_sync, R.string.setting_sync_desc, Icons.Outlined.Sync),
    Ai(R.string.agent_config_title, R.string.agent_config_description, Icons.Outlined.Dns),
    About(R.string.setting_about, null, Icons.Outlined.Info)
}

/**
 * Searchable sub-item of a settings page. The settings tab search lists these
 * (badged with their page) and jumping to one scrolls the page to [anchor],
 * which is the LazyColumn item key of the section they live in.
 */
enum class SettingsEntry(
    val destination: SettingsDestination,
    val anchor: String,
    @param:StringRes val titleRes: Int,
    @param:StringRes val descriptionRes: Int
) {
    AiConfigurations(SettingsDestination.Ai, "ai-configurations", R.string.agent_config_title, R.string.agent_config_description),
    AppearanceLanguage(
        SettingsDestination.Appearance, "appearance-language",
        R.string.desktop_appearance_language_group, R.string.setting_appearance_desc
    ),
    AppearanceTypography(
        SettingsDestination.TerminalKeys, "terminal-typography",
        R.string.desktop_appearance_typography_group, R.string.setting_terminal_shortcuts_desc
    ),
    TerminalFont(SettingsDestination.TerminalKeys, "terminal-typography",
        R.string.desktop_appearance_terminal_font, R.string.setting_terminal_shortcuts_desc),
    TerminalFallbackFont(SettingsDestination.TerminalKeys, "terminal-typography",
        R.string.desktop_appearance_cjk_fallbacks, R.string.desktop_appearance_cjk_fallbacks_desc),
    TerminalFontSize(SettingsDestination.TerminalKeys, "terminal-typography",
        R.string.font_size_label, R.string.setting_terminal_shortcuts_desc),
    TerminalLineHeight(SettingsDestination.TerminalKeys, "terminal-typography",
        R.string.font_line_height_label, R.string.setting_terminal_shortcuts_desc),
    TerminalSystemScale(SettingsDestination.TerminalKeys, "terminal-typography",
        R.string.ux_follow_font, R.string.ux_follow_font_hint),
    AppearanceTheme(
        SettingsDestination.Appearance, "appearance-theme",
        R.string.appearance_theme_group, R.string.appearance_dynamic_color_desc
    ),
    AppearanceDynamicColor(
        SettingsDestination.Appearance, "appearance-theme",
        R.string.appearance_dynamic_color, R.string.appearance_dynamic_color_desc
    ),
    ConnectionsRecent(
        SettingsDestination.Connections, "connections-recent",
        R.string.desktop_connections_recent_group, R.string.setting_connections_desc
    ),
    ConnectionsMonitor(
        SettingsDestination.Connections, "connections-monitor",
        R.string.desktop_connections_auto_monitor, R.string.desktop_connections_auto_monitor_desc
    ),
    ConnectionsKeepAlive(
        SettingsDestination.Connections, "connections-session",
        R.string.background_keep_alive, R.string.background_keep_alive_desc
    ),
    ConnectionsProxy(
        SettingsDestination.Connections, "connections-proxy",
        R.string.settings_manage_proxies, R.string.settings_proxy_hint
    ),
    ConnectionsImport(
        SettingsDestination.Connections, "connections-import",
        R.string.desktop_connections_import_group, R.string.setting_connections_desc
    ),
    VaultStatus(
        SettingsDestination.Vault, "vault-status",
        R.string.desktop_vault_status, R.string.setting_vault_desc
    ),
    VaultAutoLock(
        SettingsDestination.Vault, "vault-auto-lock",
        R.string.vault_auto_lock, R.string.setting_vault_desc
    ),
    TerminalPreview(
        SettingsDestination.TerminalKeys, "terminal-preview",
        R.string.terminal_shortcuts_preview, R.string.setting_terminal_shortcuts_desc
    ),
    TerminalQuickKeys(
        SettingsDestination.TerminalKeys, "terminal-quick",
        R.string.terminal_shortcuts_quick_group, R.string.setting_terminal_shortcuts_desc
    ),
    TerminalMoreKeys(
        SettingsDestination.TerminalKeys, "terminal-more",
        R.string.terminal_shortcuts_more_group, R.string.setting_terminal_shortcuts_desc
    ),
    SyncStatus(
        SettingsDestination.Sync, "sync-status",
        R.string.desktop_sync_status_group, R.string.setting_sync_desc
    ),
    SyncConflict(
        SettingsDestination.Sync, "sync-conflict",
        R.string.ui_resolve_conflict, R.string.setting_sync_desc
    ),
    SyncEncryption(
        SettingsDestination.Sync, "sync-encryption",
        R.string.desktop_sync_encryption_group, R.string.setting_sync_desc
    ),
    SyncProvider(
        SettingsDestination.Sync, "sync-provider",
        R.string.sync_mobile_destination, R.string.setting_sync_desc
    ),
    SyncAutomatic(
        SettingsDestination.Sync, "sync-automatic",
        R.string.ui_automatic_sync, R.string.ui_push_local_changes_automatically_check_when_returning_to_the_app
    ),
    SyncUnsafeFallback(
        SettingsDestination.Sync, "sync-capabilities",
        R.string.ui_allow_unconditional_writes_last_writer_wins,
        R.string.ui_remote_storage_cannot_guarantee_safe_writes_sync_still_compares_c
    ),
    SyncCapabilities(
        SettingsDestination.Sync, "sync-capabilities",
        R.string.sync_mobile_connection_test, R.string.sync_mobile_capability_hint
    ),
    AboutOnboarding(
        SettingsDestination.About, "about-onboarding",
        R.string.desktop_about_onboarding_group, R.string.setting_about
    ),
    AboutDiagnostics(
        SettingsDestination.About, "about-diagnostics",
        R.string.ui_diagnostics, R.string.setting_about
    )
}

/**
 * Search capsule for the settings tab: it uses the same search bar as the hosts
 * page, so the collapsed bar keeps identical bounds and only its contents swap.
 * Results cover both the pages and the concrete settings inside them; tapping one
 * opens the page and scrolls to the matching section.
 */
@Composable
fun SettingsSearchTopBar(
    onSettingClick: (SettingsDestination, String?) -> Unit = { _, _ -> },
    onMenuClick: () -> Unit = {},
    modifier: Modifier = Modifier
) {
    val context = LocalContext.current
    val versionName = remember(context) { appVersionName(context) }
    MiaominalSearchTopBar(
        placeholder = stringResource(R.string.search_settings),
        modifier = modifier,
        collapsedLeadingIcon = Icons.Outlined.Menu,
        collapsedLeadingContentDescription = stringResource(R.string.links_sidebar_toggle),
        onCollapsedLeadingClick = onMenuClick
    ) { query, collapse ->
        val matchingPages = remember(query, context, versionName) {
            SettingsDestination.entries.filter { destination ->
                val title = context.getString(destination.titleRes)
                val description = destination.descriptionRes
                    ?.let(context::getString)
                    ?: context.getString(R.string.setting_version, versionName)
                query.isEmpty() ||
                    title.contains(query, ignoreCase = true) ||
                    description.contains(query, ignoreCase = true)
            }
        }
        // Concrete settings only show up for a query: the plain tab keeps its page list.
        val matchingEntries = remember(query, context) {
            if (query.isEmpty()) {
                emptyList()
            } else {
                SettingsEntry.entries.filter { entry ->
                    context.getString(entry.titleRes).contains(query, ignoreCase = true) ||
                        context.getString(entry.descriptionRes).contains(query, ignoreCase = true)
                }
            }
        }

        LazyColumn(
            modifier = Modifier
                .fillMaxSize().wrapContentWidth().widthIn(max = 720.dp).fillMaxWidth(),
            contentPadding = PaddingValues(start = 8.dp, top = 8.dp, end = 8.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)
        ) {
            if (matchingPages.isEmpty() && matchingEntries.isEmpty()) {
                item { PageEmptyState(Icons.Outlined.Search, stringResource(R.string.search_no_settings),
                    stringResource(R.string.page_search_empty_hint), Modifier.fillParentMaxHeight()) }
            }
            items(matchingPages, key = { "page:${it.name}" }) { destination ->
                SettingsGroup {
                    SettingRow(
                        icon = destination.icon,
                        title = stringResource(destination.titleRes),
                        subtitle = destination.descriptionRes
                            ?.let { stringResource(it) }
                            ?: stringResource(R.string.setting_version, versionName),
                        onClick = {
                            collapse()
                            onSettingClick(destination, null)
                        }
                    )
                }
            }
            items(matchingEntries, key = { "entry:${it.name}" }) { entry ->
                SettingsGroup {
                    SettingRow(
                        icon = entry.destination.icon,
                        title = stringResource(entry.titleRes),
                        subtitle = stringResource(entry.descriptionRes),
                        trailing = {
                            Badge(
                                modifier = Modifier.defaultMinSize(minWidth = 24.dp, minHeight = 24.dp),
                                containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                                contentColor = MaterialTheme.colorScheme.onSurfaceVariant
                            ) {
                                Text(
                                    text = stringResource(entry.destination.titleRes),
                                    modifier = Modifier.padding(horizontal = UiSpacing.xxs),
                                    style = MaterialTheme.typography.labelMedium
                                )
                            }
                        },
                        onClick = {
                            collapse()
                            onSettingClick(entry.destination, entry.anchor)
                        }
                    )
                }
            }
        }
    }
}

@Composable
fun SettingsScreen(
    onSettingClick: (SettingsDestination) -> Unit,
    modifier: Modifier = Modifier
) {
    val context = LocalContext.current
    val versionName = remember(context) { appVersionName(context) }
    LazyColumn(
        modifier = modifier
            .fillMaxSize()
            .wrapContentWidth()
            .widthIn(max = 720.dp)
            .fillMaxWidth(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(UiSpacing.sm)
    ) {
        item {
            VaultSummaryCard()
        }

        item {
            SectionTitle(title = stringResource(R.string.settings_general))
        }

        item {
            SettingsGroup {
                SettingRow(
                    icon = Icons.Outlined.Palette,
                    title = stringResource(R.string.setting_appearance),
                    subtitle = stringResource(R.string.setting_appearance_desc),
                    onClick = { onSettingClick(SettingsDestination.Appearance) }
                )
                SettingRow(
                    icon = Icons.Outlined.Dns,
                    title = stringResource(R.string.setting_connections),
                    subtitle = stringResource(R.string.setting_connections_desc),
                    onClick = { onSettingClick(SettingsDestination.Connections) }
                )
                SettingRow(
                    icon = Icons.Outlined.Keyboard,
                    title = stringResource(R.string.setting_terminal_shortcuts),
                    subtitle = stringResource(R.string.setting_terminal_shortcuts_desc),
                    onClick = { onSettingClick(SettingsDestination.TerminalKeys) }
                )
                SettingRow(
                    icon = Icons.Outlined.Dns,
                    title = stringResource(R.string.agent_config_title),
                    subtitle = stringResource(R.string.agent_config_description),
                    onClick = { onSettingClick(SettingsDestination.Ai) }
                )
            }
        }

        item {
            SectionTitle(title = stringResource(R.string.settings_security_sync))
        }

        item {
            SettingsGroup {
                SettingRow(
                    icon = Icons.Outlined.Security,
                    title = stringResource(R.string.setting_vault),
                    subtitle = stringResource(R.string.setting_vault_desc),
                    onClick = { onSettingClick(SettingsDestination.Vault) }
                )
                SettingRow(
                    icon = Icons.Outlined.Sync,
                    title = stringResource(R.string.setting_sync),
                    subtitle = stringResource(R.string.setting_sync_desc),
                    onClick = { onSettingClick(SettingsDestination.Sync) }
                )
            }
        }

        item {
            SectionTitle(title = stringResource(R.string.settings_about))
        }

        item {
            SettingsGroup {
                SettingRow(
                    icon = Icons.Outlined.Info,
                    title = stringResource(R.string.setting_about),
                    subtitle = stringResource(R.string.setting_version, versionName),
                    onClick = { onSettingClick(SettingsDestination.About) }
                )
            }
        }
    }
}

@Composable
private fun VaultSummaryCard() {
    val vault by LocalMiaominalRepository.current.vault.collectAsState()
    val status = when (vault.mode) {
        VaultMode.Disabled -> R.string.vault_summary_disabled
        VaultMode.Locked -> R.string.vault_summary_locked
        VaultMode.Unlocked -> R.string.vault_summary_unlocked
    }
    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.primaryContainer,
            contentColor = MaterialTheme.colorScheme.onPrimaryContainer
        )
    ) {
        Column(Modifier.fillMaxWidth().padding(UiSpacing.sm), verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Outlined.Lock, contentDescription = null)
                Text(
                    stringResource(R.string.vault_summary_title),
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleMedium
                )
            }
            Badge(
                modifier = Modifier.defaultMinSize(minWidth = 24.dp, minHeight = 24.dp),
                containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                contentColor = MaterialTheme.colorScheme.primary
            ) {
                Text(
                    text = stringResource(status),
                    modifier = Modifier.padding(horizontal = UiSpacing.xxs),
                    style = MaterialTheme.typography.labelMedium
                )
            }
            Text(
                stringResource(R.string.vault_summary_desc),
                style = MaterialTheme.typography.bodyMedium
            )
        }
    }
}

@Composable
private fun SettingsGroup(content: @Composable ColumnScope.() -> Unit) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
        )
    ) {
        Column(content = content)
    }
}

@Composable
private fun SettingRow(
    icon: ImageVector,
    title: String,
    subtitle: String,
    onClick: () -> Unit = {},
    trailing: (@Composable () -> Unit)? = null
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = UiSpacing.sm, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            modifier = Modifier.size(24.dp)
        )
        Column(
            modifier = Modifier
                .weight(1f)
                .padding(start = 16.dp)
        ) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            Text(
                subtitle,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        if (trailing != null) {
            trailing()
        } else {
            Icon(
                Icons.Outlined.ChevronRight,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(24.dp)
            )
        }
    }
}
