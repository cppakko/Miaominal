package dev.akko.miaominal.ui.screens

import androidx.compose.animation.*
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.CloudUpload
import androidx.compose.material.icons.outlined.CreateNewFolder
import androidx.compose.material.icons.outlined.UploadFile
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.*
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.theme.MiaominalMotion
import dev.akko.miaominal.ui.theme.UiSpacing
import kotlin.math.roundToInt

@Composable
internal fun SftpCreateAction(onUploadFiles: () -> Unit, onUploadFolder: () -> Unit, onCreateFolder: () -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    var anchor by remember { mutableStateOf(Rect.Zero) }
    val visibility = remember { MutableTransitionState(false) }
    visibility.targetState = expanded
    val scrimAlpha by animateFloatAsState(if (expanded) 0.4f else 0f,
        tween(if (expanded) MiaominalMotion.DurationShort4 else MiaominalMotion.DurationShort2), label = "sftp-create-scrim")
    val closeLabel = stringResource(R.string.sftp_browser_close_create)
    val colors = MaterialTheme.colorScheme
    FloatingActionButton(onClick = { expanded = true },
        modifier = Modifier.testTag("sftp-create-action").onGloballyPositioned { anchor = it.boundsInWindow() },
        containerColor = colors.primaryContainer, contentColor = colors.onPrimaryContainer, shape = CircleShape) {
        Icon(Icons.Outlined.Add, stringResource(R.string.sftp_browser_add))
    }
    if (visibility.currentState || visibility.targetState) Popup(
        popupPositionProvider = SftpCreatePopupPosition,
        onDismissRequest = { expanded = false },
        properties = PopupProperties(focusable = true, clippingEnabled = false)
    ) {
        Box(Modifier.fillMaxSize().testTag("sftp-create-menu")) {
            Box(Modifier.matchParentSize().background(colors.scrim.copy(alpha = scrimAlpha))
                .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) { expanded = false }
                .testTag("sftp-create-scrim").semantics { contentDescription = closeLabel })
            Layout(modifier = Modifier.fillMaxSize(), content = {
                AnimatedVisibility(visibleState = visibility,
                    enter = fadeIn(tween(MiaominalMotion.DurationShort4)) +
                        slideInVertically(tween(MiaominalMotion.DurationShort4)) { it / 12 },
                    exit = fadeOut(tween(MiaominalMotion.DurationShort2)) +
                        slideOutVertically(tween(MiaominalMotion.DurationShort2)) { it / 12 }) {
                    Column(horizontalAlignment = Alignment.End,
                        verticalArrangement = Arrangement.spacedBy(UiSpacing.xxs)) {
                        SftpCreateMenuItem(Icons.Outlined.CreateNewFolder, stringResource(R.string.fm_new_folder)) {
                            expanded = false; onCreateFolder()
                        }
                        SftpCreateMenuItem(Icons.Outlined.UploadFile, stringResource(R.string.sftp_upload_files)) {
                            expanded = false; onUploadFiles()
                        }
                        SftpCreateMenuItem(Icons.Outlined.CloudUpload, stringResource(R.string.sftp_upload_folder)) {
                            expanded = false; onUploadFolder()
                        }
                        Spacer(Modifier.height(UiSpacing.xs))
                        FloatingActionButton(onClick = { expanded = false },
                            modifier = Modifier.testTag("sftp-create-menu-close"), shape = CircleShape,
                            containerColor = colors.primaryContainer, contentColor = colors.onPrimaryContainer) {
                            Icon(Icons.Outlined.Close, closeLabel)
                        }
                    }
                }
            }) { measurables, constraints ->
                val edge = UiSpacing.sm.roundToPx()
                val menu = measurables.single().measure(constraints.copy(minWidth = 0, minHeight = 0,
                    maxWidth = (constraints.maxWidth - edge * 2).coerceAtLeast(0),
                    maxHeight = (constraints.maxHeight - edge * 2).coerceAtLeast(0)))
                layout(constraints.maxWidth, constraints.maxHeight) {
                    // Keep the close button over the original FAB as the menu opens above it.
                    val x = if (layoutDirection == LayoutDirection.Ltr) anchor.right.roundToInt() - menu.width
                        else anchor.left.roundToInt()
                    val y = anchor.bottom.roundToInt() - menu.height
                    menu.place(x.coerceIn(0, constraints.maxWidth - menu.width),
                        y.coerceIn(0, constraints.maxHeight - menu.height))
                }
            }
        }
    }
}

@Composable
private fun SftpCreateMenuItem(icon: ImageVector, label: String, onClick: () -> Unit) {
    FilledTonalButton(onClick = onClick, shape = CircleShape,
        modifier = Modifier.heightIn(min = 56.dp).widthIn(max = 280.dp),
        contentPadding = PaddingValues(horizontal = UiSpacing.lg, vertical = UiSpacing.sm),
        colors = ButtonDefaults.filledTonalButtonColors(containerColor = MaterialTheme.colorScheme.primaryContainer,
            contentColor = MaterialTheme.colorScheme.onPrimaryContainer)) {
        Icon(icon, null, Modifier.size(24.dp))
        Spacer(Modifier.width(UiSpacing.xs))
        Text(label, style = MaterialTheme.typography.bodyLarge)
    }
}

private object SftpCreatePopupPosition : PopupPositionProvider {
    override fun calculatePosition(anchorBounds: IntRect, windowSize: IntSize, layoutDirection: LayoutDirection,
        popupContentSize: IntSize): IntOffset = IntOffset.Zero
}
