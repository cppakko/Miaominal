package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
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
import androidx.compose.material.icons.outlined.Code
import androidx.compose.material.icons.outlined.Hub
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material.icons.outlined.SwapHoriz
import androidx.compose.material.icons.outlined.VerifiedUser
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
import androidx.compose.ui.graphics.vector.ImageVector

import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.annotation.StringRes
import androidx.compose.ui.Alignment
import dev.akko.miaominal.ui.components.PageEmptyState
import dev.akko.miaominal.R
import dev.akko.miaominal.data.ForwardKind as StoreForwardKind
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.MiaominalSearchTopBar

enum class ToolDestination(
    @param:StringRes val titleRes: Int,
    @param:StringRes val descriptionRes: Int,
    val icon: ImageVector
) {
    Forward(R.string.tool_forward, R.string.tool_forward_desc, Icons.Outlined.SwapHoriz),
    Keychain(R.string.tool_keychain, R.string.tool_keychain_desc, Icons.Outlined.Key),
    Snippets(R.string.tool_snippets, R.string.tool_snippets_desc, Icons.Outlined.Code),
    Trusted(R.string.tool_trusted, R.string.tool_trusted_desc, Icons.Outlined.VerifiedUser),
    Proxies(R.string.tool_proxies, R.string.tool_proxies_desc, Icons.Outlined.Hub)
}

@Composable
fun ToolsScreen(
    onToolClick: (ToolDestination) -> Unit,
    modifier: Modifier = Modifier
) {
    LazyColumn(
        modifier = modifier
            .fillMaxSize().wrapContentWidth().widthIn(max = 840.dp).fillMaxWidth(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        items(ToolDestination.entries) { destination ->
            ToolRow(
                destination = destination,
                onClick = { onToolClick(destination) }
            )
        }
    }
}

/** One searchable item inside a tool, e.g. a forwarding rule or a managed key. */
private data class ToolSearchItem(
    val tool: ToolDestination,
    val key: String,
    val title: String,
    val subtitle: String,
    val keywords: String
)

/**
 * Flattens the data behind every tool so the tab search can find concrete items
 * ("8080", a rule name, a fingerprint, ...) and not just the tool names.
 */
@Composable
private fun rememberToolSearchItems(): List<ToolSearchItem> {
    val repository = LocalMiaominalRepository.current
    val context = LocalContext.current
    val forwards by repository.forwards.collectAsState()
    val proxies by repository.proxies.collectAsState()
    val snippets by repository.snippets.collectAsState()
    val managedKeys by repository.managedKeys.collectAsState()
    val knownHosts by repository.knownHosts.collectAsState()
    val profiles by repository.hosts.collectAsState()
    return remember(forwards, proxies, snippets, managedKeys, knownHosts, profiles, context) {
        buildList {
            forwards.forEach { rule ->
                val profile = profiles.firstOrNull { it.id == rule.profileId }
                val kind = context.getString(
                    if (rule.kind == StoreForwardKind.Local) {
                        R.string.forward_local
                    } else {
                        R.string.forward_remote
                    }
                )
                val route = "${rule.listenHost}:${rule.listenPort} → " +
                    "${rule.targetHost}:${rule.targetPort}"
                add(
                    ToolSearchItem(
                        tool = ToolDestination.Forward,
                        key = "forward:${rule.profileId}:${rule.id}",
                        title = rule.label.ifBlank { route },
                        subtitle = listOfNotNull(kind, route, profile?.name).joinToString(" · "),
                        keywords = listOf(
                            rule.label,
                            kind,
                            route,
                            profile?.name.orEmpty(),
                            profile?.host.orEmpty(),
                            profile?.port?.toString().orEmpty()
                        ).joinToString(" ")
                    )
                )
            }
            managedKeys.forEach { key ->
                add(
                    ToolSearchItem(
                        tool = ToolDestination.Keychain,
                        key = "key:${key.id}",
                        title = key.name,
                        subtitle = key.algorithm,
                        keywords = "${key.name} ${key.algorithm} ${key.publicKey}"
                    )
                )
            }
            snippets.forEach { snippet ->
                add(
                    ToolSearchItem(
                        tool = ToolDestination.Snippets,
                        key = "snippet:${snippet.id}",
                        title = snippet.description.ifBlank { snippet.packageName },
                        subtitle = listOf(snippet.packageName, snippet.language)
                            .filter { it.isNotBlank() }
                            .joinToString(" · "),
                        keywords = "${snippet.description} ${snippet.packageName} " +
                            "${snippet.language} ${snippet.script}"
                    )
                )
            }
            knownHosts.forEach { host ->
                add(
                    ToolSearchItem(
                        tool = ToolDestination.Trusted,
                        key = "known:${host.host}:${host.port}",
                        title = "${host.host}:${host.port}",
                        subtitle = listOf(host.algorithm, host.fingerprint)
                            .filter { it.isNotBlank() }
                            .joinToString(" · "),
                        keywords = "${host.host} ${host.port} ${host.algorithm} ${host.fingerprint}"
                    )
                )
            }
            proxies.forEach { proxy ->
                add(
                    ToolSearchItem(
                        tool = ToolDestination.Proxies,
                        key = "proxy:${proxy.id}",
                        title = proxy.name.ifBlank { "${proxy.host}:${proxy.port}" },
                        subtitle = listOf(proxy.protocol, "${proxy.host}:${proxy.port}", proxy.username)
                            .filter { it.isNotBlank() }
                            .joinToString(" · "),
                        keywords = "${proxy.name} ${proxy.protocol} ${proxy.host} " +
                            "${proxy.port} ${proxy.username}"
                    )
                )
            }
        }
    }
}

/**
 * Search capsule for the tools tab: it uses the same search bar as the hosts
 * page, so the collapsed bar keeps identical bounds and only its contents swap.
 */
@Composable
fun ToolsSearchTopBar(
    onToolClick: (ToolDestination, String) -> Unit = { _, _ -> },
    onMenuClick: () -> Unit = {},
    modifier: Modifier = Modifier
) {
    val context = LocalContext.current
    val items = rememberToolSearchItems()
    MiaominalSearchTopBar(
        placeholder = stringResource(R.string.search_tools),
        modifier = modifier,
        collapsedLeadingIcon = Icons.Outlined.Menu,
        collapsedLeadingContentDescription = stringResource(R.string.links_sidebar_toggle),
        onCollapsedLeadingClick = onMenuClick
    ) { query, collapse ->
        val matchingTools = remember(query, context) {
            ToolDestination.entries.filter { destination ->
                val title = context.getString(destination.titleRes)
                val description = context.getString(destination.descriptionRes)
                query.isEmpty() ||
                    title.contains(query, ignoreCase = true) ||
                    description.contains(query, ignoreCase = true)
            }
        }
        // Concrete items only show up for a query: the plain tab keeps its tool list.
        val matchingItems = remember(query, items) {
            if (query.isEmpty()) {
                emptyList()
            } else {
                items.filter { it.keywords.contains(query, ignoreCase = true) }
            }
        }

        LazyColumn(
            modifier = Modifier
                .fillMaxSize().wrapContentWidth().widthIn(max = 840.dp).fillMaxWidth(),
            contentPadding = PaddingValues(start = 8.dp, top = 8.dp, end = 8.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            if (matchingTools.isEmpty() && matchingItems.isEmpty()) {
                item { PageEmptyState(Icons.Outlined.Search, stringResource(R.string.search_no_tools),
                    stringResource(R.string.page_search_empty_hint), Modifier.fillParentMaxHeight()) }
            }
            items(matchingTools, key = { "tool:${it.name}" }) { destination ->
                ToolRow(
                    destination = destination,
                    onClick = {
                        collapse()
                        onToolClick(destination, query)
                    }
                )
            }
            items(matchingItems, key = { it.key }) { item ->
                ToolItemRow(
                    item = item,
                    onClick = {
                        collapse()
                        onToolClick(item.tool, query)
                    }
                )
            }
        }
    }
}

/** Search result for one concrete item inside a tool. */
@Composable
private fun ToolItemRow(
    item: ToolSearchItem,
    onClick: () -> Unit
) {
    Card(
        onClick = onClick,
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
        )
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Icon(
                imageVector = item.tool.icon,
                contentDescription = null,
                modifier = Modifier.size(24.dp)
            )
            Column(
                modifier = Modifier
                    .weight(1f)
                    .padding(start = 16.dp)
            ) {
                Text(
                    item.title,
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
                Text(
                    item.subtitle,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis
                )
            }
            Spacer(Modifier.size(8.dp))
            Badge(
                modifier = Modifier.defaultMinSize(minWidth = 24.dp, minHeight = 24.dp),
                containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                contentColor = MaterialTheme.colorScheme.onSurfaceVariant
            ) {
                Text(
                    text = stringResource(item.tool.titleRes),
                    modifier = Modifier.padding(horizontal = 4.dp),
                    style = MaterialTheme.typography.labelMedium
                )
            }
        }
    }
}

@Composable
private fun ToolRow(
    destination: ToolDestination,
    onClick: () -> Unit,
    modifier: Modifier = Modifier
) {
    Card(
        onClick = onClick,
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
        )
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Icon(
                imageVector = destination.icon,
                contentDescription = null,
                modifier = Modifier.size(24.dp)
            )

            Column(
                modifier = Modifier
                    .weight(1f)
                    .padding(start = 16.dp)
            ) {
                Text(
                    stringResource(destination.titleRes),
                    style = MaterialTheme.typography.titleMedium
                )
                Text(
                    stringResource(destination.descriptionRes),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            Icon(
                Icons.Outlined.ChevronRight,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(24.dp)
            )
        }
    }
}
