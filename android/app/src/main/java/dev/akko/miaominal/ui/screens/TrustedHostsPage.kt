package dev.akko.miaominal.ui.screens

import android.text.format.DateUtils
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.ExpandMore
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.VerifiedUser
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import kotlinx.coroutines.launch
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.ui.components.PageEmptyState
import dev.akko.miaominal.R
import dev.akko.miaominal.data.KnownHostRecord
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.components.PageSubtitle

@Composable
fun TrustedHostsToolPage(modifier: Modifier = Modifier) {
    val repository = LocalMiaominalRepository.current
    val scope = rememberCoroutineScope()
    val entries by repository.knownHosts.collectAsState()
    var query by rememberSaveable { mutableStateOf("") }
    var expandedKey by rememberSaveable { mutableStateOf<String?>(null) }
    var deleteKey by rememberSaveable { mutableStateOf<String?>(null) }
    val visibleEntries = entries.filter { entry ->
        query.isBlank() ||
            entry.host.contains(query, ignoreCase = true) ||
            entry.algorithm.contains(query, ignoreCase = true) ||
            entry.fingerprint.contains(query, ignoreCase = true)
    }
    val deleteTarget = entries.firstOrNull { it.recordKey() == deleteKey }

    if (deleteTarget != null) {
        AlertDialog(
            onDismissRequest = { deleteKey = null },
            icon = { Icon(Icons.Outlined.Delete, contentDescription = null) },
            title = { Text(stringResource(R.string.trusted_delete_title)) },
            text = {
                Text(stringResource(R.string.trusted_delete_message, deleteTarget.host))
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        scope.launch {
                            repository.deleteKnownHost(deleteTarget.host, deleteTarget.port)
                        }
                        deleteKey = null
                    }
                ) {
                    Text(
                        text = stringResource(R.string.tool_action_delete),
                        color = MaterialTheme.colorScheme.error
                    )
                }
            },
            dismissButton = {
                TextButton(onClick = { deleteKey = null }) {
                    Text(stringResource(R.string.tool_action_cancel))
                }
            }
        )
    }

    LazyColumn(
        modifier = modifier,
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        item { PageSubtitle(text = stringResource(R.string.tool_trusted_desc)) }
        item {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                leadingIcon = {
                    Icon(Icons.Outlined.Search, contentDescription = null)
                },
                placeholder = { Text(stringResource(R.string.trusted_search)) }
            )
        }
        if (entries.isEmpty()) {
            item {
                PageEmptyState(
                        title = stringResource(R.string.page_empty_title),
                        modifier = Modifier.fillParentMaxHeight(0.65f),
                    icon = Icons.Outlined.VerifiedUser,
                    text = stringResource(R.string.trusted_empty)
                )
            }
        } else {
            item {
                Text(
                    text = stringResource(R.string.trusted_count, entries.size),
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            if (visibleEntries.isEmpty()) {
                item {
                    PageEmptyState(
                        title = stringResource(R.string.page_empty_title),
                        modifier = Modifier.fillParentMaxHeight(0.65f),
                        icon = Icons.Outlined.Search,
                        text = stringResource(R.string.trusted_no_match)
                    )
                }
            } else {
                items(visibleEntries, key = { it.recordKey() }) { entry ->
                    TrustedHostCard(
                        entry = entry,
                        expanded = expandedKey == entry.recordKey(),
                        onToggleExpanded = {
                            expandedKey = if (expandedKey == entry.recordKey()) null else entry.recordKey()
                        },
                        onDelete = { deleteKey = entry.recordKey() }
                    )
                }
            }
        }
    }
}

@Composable
private fun TrustedHostCard(
    entry: KnownHostRecord,
    expanded: Boolean,
    onToggleExpanded: () -> Unit,
    onDelete: () -> Unit
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLowest
        ),
        shape = MaterialTheme.shapes.large
    ) {
        Column(modifier = Modifier.clickable(onClick = onToggleExpanded)) {
            ListItem(
                headlineContent = {
                    Text(
                        text = "${entry.host}:${entry.port}",
                        style = MaterialTheme.typography.titleMedium.copy(
                            fontFamily = FontFamily.Monospace
                        ),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                },
                supportingContent = {
                    Text(
                        text = entry.algorithm,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                },
                leadingContent = {
                    Surface(
                        modifier = Modifier.size(38.dp),
                        shape = CircleShape,
                        color = MaterialTheme.colorScheme.secondaryContainer,
                        contentColor = MaterialTheme.colorScheme.onSecondaryContainer
                    ) {
                        Icon(
                            imageVector = Icons.Outlined.VerifiedUser,
                            contentDescription = null,
                            modifier = Modifier.padding(8.dp)
                        )
                    }
                },
                trailingContent = {
                    Icon(
                        imageVector = Icons.Outlined.ExpandMore,
                        contentDescription = null,
                        modifier = Modifier.graphicsLayer {
                            rotationZ = if (expanded) 180f else 0f
                        }
                    )
                },
                colors = ListItemDefaults.colors(containerColor = Color.Transparent)
            )
            AnimatedVisibility(visible = expanded) {
                Column(modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 12.dp)) {
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                    Spacer(Modifier.padding(top = 8.dp))
                    Text(
                        text = stringResource(R.string.trusted_fingerprint),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Text(
                        text = entry.fingerprint,
                        style = MaterialTheme.typography.bodySmall.copy(
                            fontFamily = FontFamily.Monospace
                        )
                    )
                    if (entry.addedAt != null) {
                        Spacer(Modifier.padding(top = 8.dp))
                        Text(
                            text = stringResource(R.string.trusted_added) + " · " +
                                DateUtils.getRelativeTimeSpanString(
                                    entry.addedAt,
                                    System.currentTimeMillis(),
                                    DateUtils.DAY_IN_MILLIS
                                ).toString(),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    Row(horizontalArrangement = Arrangement.End, modifier = Modifier.fillMaxWidth()) {
                        TextButton(onClick = onDelete) {
                            Icon(
                                imageVector = Icons.Outlined.Delete,
                                contentDescription = null,
                                modifier = Modifier.size(18.dp)
                            )
                            Spacer(Modifier.width(6.dp))
                            Text(
                                text = stringResource(R.string.tool_action_delete),
                                color = MaterialTheme.colorScheme.error
                            )
                        }
                    }
                }
            }
        }
    }
}

private fun KnownHostRecord.recordKey(): String = "$host:$port"
