package dev.akko.miaominal.ui.screens

import dev.akko.miaominal.ui.theme.UiSpacing
import androidx.compose.foundation.layout.wrapContentWidth

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.ui.components.PageEmptyState
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.components.HostRow
import dev.akko.miaominal.ui.components.SectionTitle
import dev.akko.miaominal.ui.model.HostProfileUi

private enum class HostFilter { All, Favorites, Connected }

@Composable
fun HostsScreen(
    hosts: List<HostProfileUi>,
    recentHosts: List<HostProfileUi>,
    expandedHostId: String?,
    onExpandedChange: (String?) -> Unit,
    onToggleFavorite: (HostProfileUi) -> Unit,
    onAddHost: () -> Unit,
    onConnectHost: (HostProfileUi) -> Unit = {},
    onEditHost: (HostProfileUi) -> Unit = {},
    onSftpHost: (HostProfileUi) -> Unit = {},
    onAgentHost: ((HostProfileUi) -> Unit)? = null,
    modifier: Modifier = Modifier
) {
    var selectedGroup by rememberSaveable { mutableStateOf<String?>(null) }
    var selectedFilter by rememberSaveable { mutableStateOf(HostFilter.All) }
    val listState = rememberLazyListState()
    val groups = remember(hosts) {
        hosts.map { it.group }.filter { it.isNotBlank() }.distinct().sorted()
    }
    val activeGroup = selectedGroup?.takeIf { it in groups && groups.size > 1 }
    LaunchedEffect(activeGroup) { selectedGroup = activeGroup }
    val visibleHosts = remember(hosts, activeGroup, selectedFilter) {
        hosts
            .filter { activeGroup == null || it.group == activeGroup }
            .filter {
                when (selectedFilter) {
                    HostFilter.All -> true
                    HostFilter.Favorites -> it.isFavorite
                    HostFilter.Connected -> it.online
                }
            }
            .sortedWith(
                compareByDescending<HostProfileUi> { it.isFavorite }
                    .thenByDescending { it.online }
                    .thenBy { it.name.lowercase() }
            )
    }

    LazyColumn(
        modifier = modifier
            .fillMaxSize().wrapContentWidth().widthIn(max = 720.dp).fillMaxWidth(),
        state = listState,
        contentPadding = PaddingValues(start = 8.dp, top = 8.dp, end = 8.dp, bottom = 112.dp),
        verticalArrangement = Arrangement.spacedBy(UiSpacing.xs)
    ) {
        if (hosts.isEmpty()) {
            item { QuickConnectCard(onAddHost = onAddHost, modifier = Modifier.fillParentMaxHeight()) }
        } else {
            if (recentHosts.isNotEmpty()) {
                item { SectionTitle(title = stringResource(R.string.recent_hosts)) }
                item { RecentHostsRow(recentHosts = recentHosts, onConnectHost = onConnectHost) }
            }
            item(key = "host-filters") {
                HostFilterRow(
                    selectedFilter = selectedFilter,
                    onSelectFilter = { selectedFilter = it },
                    groups = groups,
                    selectedGroup = activeGroup,
                    onSelectGroup = { selectedGroup = it }
                )
            }
            if (visibleHosts.isEmpty()) {
                item(key = "host-filter-empty") {
                    PageEmptyState(
                        icon = Icons.Outlined.Search,
                        title = stringResource(R.string.hosts_filter_empty),
                        text = stringResource(R.string.hosts_filter_empty_hint),
                        modifier = Modifier.fillParentMaxHeight(0.65f),
                        action = {
                            TextButton(onClick = { selectedFilter = HostFilter.All; selectedGroup = null }) {
                                Text(stringResource(R.string.hosts_clear_filters))
                            }
                        }
                    )
                }
            }
            items(visibleHosts, key = { it.id }) { host ->
                HostRow(
                    host = host,
                    expanded = expandedHostId == host.id,
                    onExpandedChange = {
                        onExpandedChange(if (expandedHostId == host.id) null else host.id)
                    },
                    onConnect = { onConnectHost(host) },
                    onAgent = onAgentHost?.let { open -> { open(host) } },
                    onSftp = { onSftpHost(host) },
                    onEdit = { onEditHost(host) },
                    onToggleFavorite = { onToggleFavorite(host) }
                )
            }
        }
    }
}

@Composable
private fun HostFilterRow(
    selectedFilter: HostFilter,
    onSelectFilter: (HostFilter) -> Unit,
    groups: List<String>,
    selectedGroup: String?,
    onSelectGroup: (String?) -> Unit
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs),
        verticalAlignment = Alignment.CenterVertically
    ) {
        LazyRow(
            modifier = Modifier.weight(1f),
            horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)
        ) {
            items(HostFilter.entries, key = { it.name }) { filter ->
                val label = when (filter) {
                    HostFilter.All -> R.string.hosts_filter_all
                    HostFilter.Favorites -> R.string.hosts_filter_favorites
                    HostFilter.Connected -> R.string.hosts_filter_connected
                }
                FilterChip(
                    selected = selectedFilter == filter,
                    onClick = { onSelectFilter(filter) },
                    label = { Text(stringResource(label)) }
                )
            }
        }
        if (groups.size > 1) {
            GroupFilterPicker(groups, selectedGroup, onSelectGroup)
        }
    }
}

@Composable
private fun RecentHostsRow(
    recentHosts: List<HostProfileUi>,
    onConnectHost: (HostProfileUi) -> Unit
) {
    LazyRow(horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs)) {
        items(recentHosts, key = { "recent-${it.id}" }) { host ->
            Surface(
                onClick = { onConnectHost(host) },
                color = MaterialTheme.colorScheme.secondaryContainer,
                contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
                shape = MaterialTheme.shapes.large
            ) {
                Row(
                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Icon(
                        imageVector = Icons.Outlined.Terminal,
                        contentDescription = null,
                        modifier = Modifier.size(18.dp)
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        text = "${host.name} · ${host.username}@${host.address}",
                        style = MaterialTheme.typography.labelLarge.copy(fontFamily = FontFamily.Monospace),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                }
            }
        }
    }
}

@Composable
@OptIn(ExperimentalMaterial3Api::class)
private fun GroupFilterPicker(
    groups: List<String>,
    selectedGroup: String?,
    onSelect: (String?) -> Unit
) {
    var showSheet by remember { mutableStateOf(false) }
    FilterChip(
        selected = selectedGroup != null,
        onClick = { showSheet = true },
        leadingIcon = {
            Icon(Icons.Outlined.Folder, contentDescription = null, modifier = Modifier.size(18.dp))
        },
        label = {
            Text(
                text = selectedGroup ?: stringResource(R.string.profile_group),
                modifier = Modifier.widthIn(max = 112.dp),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        },
        trailingIcon = {
            Icon(Icons.Filled.ArrowDropDown, contentDescription = null, modifier = Modifier.size(18.dp))
        }
    )
    if (showSheet) {
        val sheetColor = MaterialTheme.colorScheme.surfaceContainerLow
        ModalBottomSheet(
            onDismissRequest = { showSheet = false },
            containerColor = sheetColor
        ) {
            Column(modifier = Modifier.fillMaxWidth().padding(bottom = 16.dp)) {
                Text(
                    text = stringResource(R.string.hosts_group_filter),
                    modifier = Modifier.padding(horizontal = UiSpacing.md, vertical = UiSpacing.xs),
                    style = MaterialTheme.typography.titleLarge
                )
                Column(
                    modifier = Modifier
                        .heightIn(max = 480.dp)
                        .verticalScroll(rememberScrollState())
                        .selectableGroup()
                ) {
                    (listOf<String?>(null) + groups).forEach { group ->
                        val isSelected = selectedGroup == group
                        ListItem(
                            modifier = Modifier.fillMaxWidth().selectable(
                                selected = isSelected,
                                role = Role.RadioButton,
                                onClick = { onSelect(group); showSheet = false }
                            ),
                            headlineContent = {
                                Text(
                                    text = group ?: stringResource(R.string.hosts_filter_all_groups),
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis
                                )
                            },
                            trailingContent = {
                                RadioButton(selected = isSelected, onClick = null)
                            },
                            colors = ListItemDefaults.colors(containerColor = sheetColor)
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun QuickConnectCard(onAddHost: () -> Unit, modifier: Modifier = Modifier) {
    PageEmptyState(Icons.Outlined.Terminal, stringResource(R.string.quick_connect_title),
        stringResource(R.string.quick_connect_body), modifier, action = {
            TextButton(onClick = onAddHost) {
                Icon(Icons.Outlined.Add, null, Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text(stringResource(R.string.quick_connect_action))
            }
        })
}