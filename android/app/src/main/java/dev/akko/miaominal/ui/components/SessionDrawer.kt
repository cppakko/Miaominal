package dev.akko.miaominal.ui.components

import androidx.compose.animation.core.animate
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.material3.DrawerDefaults
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import kotlin.math.roundToInt

/**
 * Modal session drawer that can only be *opened* from the top-bar button.
 *
 * `ModalNavigationDrawer` exposes a single `gesturesEnabled` flag, and turning
 * it off also disables swipe-to-close and the scrim tap (the scrim's click is
 * gated on the same flag). This variant keeps both closing gestures — swipe
 * left on the sheet, or tap anywhere outside it — while an edge swipe can never
 * pull the drawer in (the accidental opens while scrolling).
 */
@Composable
fun SessionDrawer(
    open: Boolean,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
    drawerContent: @Composable ColumnScope.() -> Unit,
    content: @Composable () -> Unit
) {
    val sheetWidthPx = with(LocalDensity.current) { DrawerDefaults.MaximumDrawerWidth.toPx() }
    // Horizontal offset of the sheet: 0 = open, -sheetWidthPx = closed.
    var offsetPx by remember { mutableFloatStateOf(if (open) 0f else -sheetWidthPx) }

    LaunchedEffect(open) {
        animate(offsetPx, if (open) 0f else -sheetWidthPx) { value, _ -> offsetPx = value }
    }

    Box(modifier.fillMaxSize()) {
        content()

        val progress = (-offsetPx / sheetWidthPx).coerceIn(0f, 1f)
        if (progress < 1f) {
            Box(
                Modifier
                    .fillMaxSize()
                    .background(
                        DrawerDefaults.scrimColor.copy(
                            alpha = DrawerDefaults.scrimColor.alpha * (1f - progress)
                        )
                    )
                    .clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                        onClick = onClose
                    )
            )
        }

        // Composed while open or still animating; a fully closed drawer does not
        // exist in the tree, so it can never intercept touches.
        if (open || progress < 1f) {
            ModalDrawerSheet(
                modifier = Modifier
                    .offset { IntOffset(offsetPx.roundToInt(), 0) }
                    .draggable(
                        orientation = Orientation.Horizontal,
                        state = rememberDraggableState { delta ->
                            offsetPx = (offsetPx + delta).coerceIn(-sheetWidthPx, 0f)
                        },
                        onDragStopped = { velocity ->
                            if (offsetPx < -sheetWidthPx * 0.5f || velocity < -600f) {
                                // The caller flips `open`, which animates the rest.
                                onClose()
                            } else {
                                animate(offsetPx, 0f) { value, _ -> offsetPx = value }
                            }
                        }
                    )
            ) {
                drawerContent()
            }
        }
    }
}
