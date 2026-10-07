package dev.akko.miaominal.ui.components

import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.stateDescription
import dev.akko.miaominal.ui.theme.successColor

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Psychology
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Link
import androidx.compose.material.icons.outlined.Terminal
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R

enum class OpenedLinkKind {
    Ssh,
    Sftp,
    Agent
}

enum class OpenedLinkStatus {
    Connected,
    Active,
    Error,
    Idle
}

data class OpenedLinkUi(
    /** Unique drawer key (`ssh:<id>` / `sftp:<id>`); SSH and SFTP ids can collide. */
    val id: String,
    /** Backend connection id of the session this entry refers to. */
    val connectionId: Long,
    val title: String,
    val subtitle: String,
    val kind: OpenedLinkKind,
    val status: OpenedLinkStatus
)

/**
 * MD3 modal navigation drawer listing currently opened links.
 *
 * Follows the navigation drawer spec: 360dp modal drawer on `surfaceContainerLow`
 * with `CornerLargeEnd` shape, and destinations built on the NavigationDrawerItem
 * tokens (56dp pill rows, 24dp icons, `labelLarge` labels, selected state on
 * `secondaryContainer`).
 */
@Composable
fun ColumnScope.OpenedLinksDrawer(
    links: List<OpenedLinkUi>,
    selectedLinkId: String?,
    onClose: () -> Unit,
    onSelect: (OpenedLinkUi) -> Unit,
    onCloseLink: (OpenedLinkUi) -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = 16.dp, top = 16.dp, end = 8.dp, bottom = 12.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Surface(
            modifier = Modifier.size(40.dp),
            shape = CircleShape,
            color = MaterialTheme.colorScheme.secondaryContainer,
            contentColor = MaterialTheme.colorScheme.onSecondaryContainer
        ) {
            Box(contentAlignment = Alignment.Center) {
                Icon(Icons.Outlined.Link, contentDescription = null)
            }
        }
        Spacer(Modifier.width(12.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = stringResource(R.string.links_sidebar_title),
                style = MaterialTheme.typography.titleLarge
            )
            Text(
                text = stringResource(R.string.links_sidebar_count, links.size),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        IconButton(onClick = onClose) {
            Icon(
                imageVector = Icons.Outlined.Close,
                contentDescription = stringResource(R.string.links_sidebar_close_drawer)
            )
        }
    }

    LazyColumn(
        modifier = Modifier.weight(1f),
        contentPadding = PaddingValues(start = 12.dp, end = 12.dp, bottom = 12.dp)
    ) {
        if (links.isEmpty()) {
            item {
                Text(
                    text = stringResource(R.string.links_sidebar_empty),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp)
                )
            }
        }
        items(links, key = { it.id }) { link ->
            OpenedLinkDrawerItem(
                link = link,
                selected = link.id == selectedLinkId,
                onClick = { onSelect(link) },
                onClose = { onCloseLink(link) }
            )
        }
    }
}

@Composable
private fun OpenedLinkDrawerItem(
    link: OpenedLinkUi,
    selected: Boolean,
    onClick: () -> Unit,
    onClose: () -> Unit
) {
    val icon = when (link.kind) {
        OpenedLinkKind.Ssh -> Icons.Outlined.Terminal
        OpenedLinkKind.Sftp -> Icons.Outlined.Folder
        OpenedLinkKind.Agent -> Icons.Outlined.Psychology
    }
    val statusColor = when (link.status) {
        OpenedLinkStatus.Connected -> successColor
        OpenedLinkStatus.Active -> MaterialTheme.colorScheme.primary
        OpenedLinkStatus.Error -> MaterialTheme.colorScheme.error
        OpenedLinkStatus.Idle -> MaterialTheme.colorScheme.outline
    }
    val statusLabel = stringResource(when (link.status) {
        OpenedLinkStatus.Connected -> R.string.ux_connected
        OpenedLinkStatus.Active -> R.string.ux_active
        OpenedLinkStatus.Error -> R.string.ux_failed
        OpenedLinkStatus.Idle -> R.string.ux_waiting
    })
    val contentColor = if (selected) {
        MaterialTheme.colorScheme.onSecondaryContainer
    } else {
        MaterialTheme.colorScheme.onSurfaceVariant
    }

    Surface(
        onClick = onClick,
        modifier = Modifier
            .fillMaxWidth()
            .semantics { role = Role.Tab; this.selected = selected; stateDescription = statusLabel },
        shape = CircleShape,
        color = if (selected) {
            MaterialTheme.colorScheme.secondaryContainer
        } else {
            Color.Transparent
        }
    ) {
        Row(
            modifier = Modifier
                .heightIn(min = 56.dp)
                .padding(start = 16.dp, top = 8.dp, end = 4.dp, bottom = 8.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                modifier = Modifier.size(24.dp),
                tint = contentColor
            )
            Spacer(Modifier.width(12.dp))
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp)
            ) {
                Text(
                    text = link.title,
                    style = MaterialTheme.typography.labelLarge,
                    color = contentColor,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
                Text(
                    text = "${link.kind.name} · $statusLabel · ${link.subtitle}",
                    style = MaterialTheme.typography.bodySmall.copy(
                        fontFamily = FontFamily.Monospace
                    ),
                    color = contentColor,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
            }
            Spacer(Modifier.width(12.dp))
            Box(
                modifier = Modifier
                    .size(10.dp)
                    .clip(CircleShape)
                    .background(statusColor)
            )
            IconButton(onClick = onClose) {
                Icon(
                    imageVector = Icons.Outlined.Close,
                    contentDescription = stringResource(R.string.ux_close_named, link.title),
                    modifier = Modifier.size(20.dp)
                )
            }
        }
    }
}
