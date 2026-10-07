package dev.akko.miaominal.ui.screens

import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.fillMaxWidth

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.ui.components.PageEmptyState
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.components.HostRow
import dev.akko.miaominal.ui.components.MiaominalSearchTopBar
import dev.akko.miaominal.ui.model.HostProfileUi

@Composable
fun HostSearchTopBar(
    hosts: List<HostProfileUi>,
    onConnectHost: (HostProfileUi) -> Unit = {},
    onEditHost: (HostProfileUi) -> Unit = {},
    onSftpHost: (HostProfileUi) -> Unit = {},
    onAgentHost: ((HostProfileUi) -> Unit)? = null,
    onToggleFavorite: (HostProfileUi) -> Unit,
    onMenuClick: () -> Unit = {},
    modifier: Modifier = Modifier
) {
    var expandedHostId by rememberSaveable { mutableStateOf<String?>(null) }

    MiaominalSearchTopBar(
        placeholder = stringResource(R.string.search_hosts),
        modifier = modifier,
        collapsedLeadingIcon = Icons.Outlined.Menu,
        collapsedLeadingContentDescription = stringResource(R.string.links_sidebar_toggle),
        onCollapsedLeadingClick = onMenuClick
    ) { query, collapse ->
        val filteredHosts = remember(query, hosts) {
            if (query.isEmpty()) {
                hosts
            } else {
                hosts.filter { host ->
                    host.name.contains(query, ignoreCase = true) ||
                        host.address.contains(query, ignoreCase = true) ||
                        host.username.contains(query, ignoreCase = true) ||
                        host.group.contains(query, ignoreCase = true) ||
                        host.tags.any { it.contains(query, ignoreCase = true) }
                }
            }
        }

        LazyColumn(
            modifier = Modifier.fillMaxSize().wrapContentWidth().widthIn(max = 720.dp).fillMaxWidth(),
            contentPadding = PaddingValues(
                start = 8.dp,
                top = 8.dp,
                end = 8.dp,
                bottom = 16.dp
            ),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            if (filteredHosts.isEmpty()) {
                item { PageEmptyState(Icons.Outlined.Search, stringResource(R.string.search_no_hosts),
                    stringResource(R.string.page_search_empty_hint), Modifier.fillParentMaxHeight()) }
            }
            items(filteredHosts, key = { it.id }) { host ->
                HostRow(
                    host = host,
                    onToggleFavorite = { onToggleFavorite(host) },
                    expanded = expandedHostId == host.id,
                    onExpandedChange = {
                        expandedHostId = if (expandedHostId == host.id) null else host.id
                    },
                    onConnect = {
                        collapse()
                        onConnectHost(host)
                    },
                    onAgent = onAgentHost?.let { open -> { collapse(); open(host) } },
                    onSftp = {
                        collapse()
                        onSftpHost(host)
                    },
                    onEdit = {
                        collapse()
                        onEditHost(host)
                    }
                )
            }
        }
    }
}
