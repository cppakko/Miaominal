package dev.akko.miaominal.ui.components

import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.res.stringResource
import dev.akko.miaominal.R

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.SearchBarDefaults
import androidx.compose.material3.TopSearchBar
import androidx.compose.material3.rememberSearchBarState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.ui.model.HostProfileUi
import dev.akko.miaominal.ui.theme.successColor

/** Height of the session top bar shared by the terminal and SFTP screens. */
private val SessionTopBarContentHeight
    @Composable get() = with(LocalDensity.current) {
        maxOf(
            56.dp,
            MaterialTheme.typography.titleMedium.lineHeight.toDp() +
                MaterialTheme.typography.bodySmall.lineHeight.toDp() + 14.dp
        )
    }

/** Total occupied height, including the same 8dp top/bottom spacing as Hosts' search bar. */
val SessionTopBarHeight
    @Composable get() = SessionTopBarContentHeight + 16.dp

/** The shared shape used by the Hosts search bar and every session top bar. */
val MiaominalTopBarShape = CircleShape

/**
 * Shared top bar capsule for every page: the hosts search bar, the app-level
 * tabs (tools, settings) and the session surfaces (SSH, SFTP, AI). It uses the
 * same Material3 TopSearchBar container, shape and built-in 8dp margins as the
 * hosts search bar, so the bar keeps identical bounds while its contents swap.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionTopBarCapsule(
    modifier: Modifier = Modifier,
    content: @Composable RowScope.() -> Unit
) {
    val state = rememberSearchBarState()
    TopSearchBar(
        state = state,
        inputField = {
            Row(
                modifier = Modifier
                    .widthIn(max = 720.dp)
                    .fillMaxWidth()
                    .height(SessionTopBarContentHeight)
                    .padding(horizontal = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                content = content
            )
        },
        modifier = modifier.fillMaxWidth(),
        shape = MiaominalTopBarShape,
        colors = SearchBarDefaults.colors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLowest,
            dividerColor = Color.Transparent
        ),
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
        windowInsets = WindowInsets(0, 0, 0, 0)
    )
}

/**
 * Host identity block of a session top bar: status dot, host name and the
 * `user@address:port` line.
 *
 * Shared so the terminal and SFTP headers stay visually identical: both are
 * first-class session surfaces over the same authenticated host.
 */
@Composable
fun SessionIdentity(
    profile: HostProfileUi,
    connected: Boolean,
    modifier: Modifier = Modifier
) {
    val statusLabel = stringResource(if (connected) R.string.ux_connected else R.string.ux_waiting)
    Column(modifier.semantics { stateDescription = statusLabel }, verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier
                    .size(6.dp)
                    .background(
                        if (connected) successColor else MaterialTheme.colorScheme.onSurfaceVariant,
                        CircleShape
                    )
            )
            Spacer(Modifier.width(8.dp))
            Text(
                text = profile.name,
                color = MaterialTheme.colorScheme.onSurface,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }
        Text(
            text = "${profile.username}@${profile.address}:${profile.port}",
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis
        )
    }
}
