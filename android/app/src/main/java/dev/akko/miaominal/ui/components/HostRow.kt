package dev.akko.miaominal.ui.components

import dev.akko.miaominal.ui.theme.UiSpacing
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.material3.AlertDialog
import dev.akko.miaominal.ui.theme.ExpressiveMotion

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.outlined.StarBorder
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.Badge
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.TextButton
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.model.HostProfileUi
import dev.akko.miaominal.ui.theme.successColor
import androidx.compose.ui.res.stringResource

private val HostCardEasing = CubicBezierEasing(0.05f, 0.7f, 0.1f, 1f)

@Composable
fun HostRow(
    host: HostProfileUi,
    expanded: Boolean,
    onExpandedChange: () -> Unit,
    onConnect: () -> Unit,
    onSftp: () -> Unit = {},
    onAgent: (() -> Unit)? = null,
    onEdit: () -> Unit = {},
    onToggleFavorite: () -> Unit = {},
    modifier: Modifier = Modifier
) {
    var showTags by remember { mutableStateOf(false) }
    val connectLabel = stringResource(R.string.ux_connect_named, host.name)
    val favoriteLabel = stringResource(if (host.isFavorite) R.string.ux_favorite else R.string.ux_not_favorite)
    val expandedLabel = stringResource(if (expanded) R.string.action_collapse else R.string.action_expand)
    val cardColor = MaterialTheme.colorScheme.surfaceContainerLowest
    val chevronRotation by animateFloatAsState(
        targetValue = if (expanded) 180f else 0f,
        animationSpec = ExpressiveMotion.stateChange(),
        label = "host-expand-chevron"
    )

    if (showTags) AlertDialog(onDismissRequest = { showTags = false },
        title = { Text(stringResource(R.string.ux_more_tags, host.tags.size)) },
        text = { Text(host.tags.joinToString(" · ")) },
        confirmButton = { TextButton(onClick = { showTags = false }) { Text(stringResource(android.R.string.ok)) } })
    Box {
        Card(
            // Connecting and expanding are independent actions in every state.
            onClick = onConnect,
            modifier = modifier.fillMaxWidth().semantics { onClick(label = connectLabel, action = null) },
            colors = CardDefaults.cardColors(containerColor = cardColor),
            shape = MaterialTheme.shapes.large
        ) {
            Column(
                modifier = Modifier.padding(UiSpacing.sm)
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Box {
                        Surface(
                            modifier = Modifier.size(42.dp),
                            color = MaterialTheme.colorScheme.secondaryContainer,
                            contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
                            shape = CircleShape
                        ) {
                            Box(contentAlignment = Alignment.Center) {
                                Icon(
                                    imageVector = Icons.Outlined.Terminal,
                                    contentDescription = null,
                                    modifier = Modifier.size(22.dp)
                                )
                            }
                        }
                        Box(
                            modifier = Modifier
                                .align(Alignment.BottomEnd)
                                .offset(x = 2.dp, y = 2.dp)
                                .size(12.dp)
                                .clip(CircleShape)
                                .background(
                                    if (host.online) successColor
                                    else MaterialTheme.colorScheme.outline
                                )
                                .border(2.dp, cardColor, CircleShape)
                        )
                    }
                    Spacer(Modifier.width(12.dp))
                    Column(
                        modifier = Modifier.weight(1f),
                        verticalArrangement = Arrangement.spacedBy(2.dp)
                    ) {
                        Text(
                            text = host.name,
                            style = MaterialTheme.typography.titleMedium,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                        Text(
                            text = "${host.username}@${host.address}:${host.port}",
                            style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                        if (host.tags.isNotEmpty()) {
                            FlowRow(
                                horizontalArrangement = Arrangement.spacedBy(6.dp),
                                verticalArrangement = Arrangement.spacedBy(UiSpacing.xxs)
                            ) {
                                host.tags.take(3).forEach { tag ->
                                    Badge(
                                        modifier = Modifier.defaultMinSize(
                                            minWidth = 24.dp,
                                            minHeight = 24.dp
                                        ),
                                        containerColor = MaterialTheme.colorScheme.secondaryContainer,
                                        contentColor = MaterialTheme.colorScheme.onSecondaryContainer
                                    ) {
                                        Text(
                                            text = tag,
                                            modifier = Modifier.padding(horizontal = UiSpacing.xxs),
                                            style = MaterialTheme.typography.labelMedium
                                        )
                                    }
                                }
                                if (host.tags.size > 3) TextButton(onClick = { showTags = true }) {
                                    Text("+${host.tags.size - 3}")
                                }
                            }
                        }
                    }
                    IconButton(onClick = onToggleFavorite,
                        modifier = Modifier.semantics { stateDescription = favoriteLabel }) {
                        Icon(
                            imageVector = if (host.isFavorite) {
                                Icons.Filled.Star
                            } else {
                                Icons.Outlined.StarBorder
                            },
                            contentDescription = stringResource(R.string.profile_favorite),
                            tint = if (host.isFavorite) {
                                MaterialTheme.colorScheme.primary
                            } else {
                                MaterialTheme.colorScheme.onSurfaceVariant
                            }
                        )
                    }
                    IconButton(onClick = onExpandedChange, modifier = Modifier.semantics { stateDescription = expandedLabel }) {
                        Icon(
                            imageVector = Icons.Filled.KeyboardArrowDown,
                            contentDescription = stringResource(
                                if (expanded) R.string.action_collapse
                                else R.string.action_expand
                            ),
                            modifier = Modifier.graphicsLayer { rotationZ = chevronRotation }
                        )
                    }
                }

                AnimatedVisibility(
                    visible = expanded,
                    enter = expandVertically(
                        animationSpec = ExpressiveMotion.stateChange(),
                        expandFrom = Alignment.Top,
                        clip = true
                    ) + fadeIn(
                        animationSpec = tween(durationMillis = 220, delayMillis = 40)
                    ),
                    exit = shrinkVertically(
                        animationSpec = ExpressiveMotion.stateChange(),
                        shrinkTowards = Alignment.Top,
                        clip = true
                    ) + fadeOut(
                        animationSpec = tween(durationMillis = 180, easing = HostCardEasing)
                    )
                ) {
                    Column {
                        Spacer(Modifier.size(14.dp))
                        if (host.group.isNotBlank()) {
                            Spacer(Modifier.size(10.dp))
                            FlowRow(
                                horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs),
                                verticalArrangement = Arrangement.spacedBy(6.dp)
                            ) {
                                Badge(
                                    modifier = Modifier.defaultMinSize(
                                        minWidth = 24.dp,
                                        minHeight = 24.dp
                                    ),
                                    containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                                    contentColor = MaterialTheme.colorScheme.onSurfaceVariant
                                ) {
                                    Text(
                                        text = host.group,
                                        modifier = Modifier.padding(horizontal = UiSpacing.xxs),
                                        style = MaterialTheme.typography.labelMedium
                                    )
                                }
                            }
                        }
                        Spacer(Modifier.size(14.dp))
                        FlowRow(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(UiSpacing.xs),
                            verticalArrangement = Arrangement.spacedBy(UiSpacing.xxs)
                        ) {
                            Button(
                                onClick = onConnect
                            ) {
                                Text(stringResource(R.string.action_connect))
                            }
                            OutlinedButton(
                                onClick = onSftp
                            ) {
                                Text(stringResource(R.string.action_sftp))
                            }
                            onAgent?.let { openAgent ->
                                OutlinedButton(onClick = openAgent) {
                                    Text(stringResource(R.string.ui_ai_session))
                                }
                            }
                            TextButton(
                                onClick = onEdit
                            ) {
                                Text(stringResource(R.string.action_edit))
                            }
                        }
                    }
                }
            }
        }
    }
}
