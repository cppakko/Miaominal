package dev.akko.miaominal.ui.components

import android.app.Activity
import androidx.compose.foundation.layout.*
import androidx.compose.material3.adaptive.currentWindowAdaptiveInfo
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.geometry.Offset
import androidx.window.layout.FoldingFeature
import androidx.window.layout.WindowInfoTracker
import dev.akko.miaominal.ui.model.supportsTwoPanes

val LocalTwoPaneAvailable = staticCompositionLocalOf { false }
val LocalWorkspaceWidth = staticCompositionLocalOf { 360 }
val LocalWorkspaceHeight = staticCompositionLocalOf { 800 }

/** Insets separating folds from the usable workspace, keeping controls off the hinge. */
@Composable
fun AdaptiveWorkspace(content: @Composable () -> Unit) {
    currentWindowAdaptiveInfo() // Subscribe to Material adaptive posture/window updates.
    val activity = LocalContext.current as? Activity
    val layoutInfo = remember(activity) { activity?.let { WindowInfoTracker.getOrCreate(it).windowLayoutInfo(it) } }
        ?.collectAsState(initial = null)?.value
    val fold = layoutInfo?.displayFeatures?.filterIsInstance<FoldingFeature>()?.firstOrNull { it.isSeparating }
    val density = LocalDensity.current
    var origin by remember { mutableStateOf(Offset.Zero) }
    BoxWithConstraints(Modifier.fillMaxSize().onGloballyPositioned { origin = it.positionInWindow() }) {
        val widthPx = with(density) { maxWidth.roundToPx() }
        val heightPx = with(density) { maxHeight.roundToPx() }
        val region = dev.akko.miaominal.ui.model.safeWorkspaceRegion(widthPx, heightPx,
            fold?.bounds?.let { bounds -> dev.akko.miaominal.ui.model.WorkspaceRegion(
                bounds.left - origin.x.toInt(), bounds.top - origin.y.toInt(),
                bounds.right - origin.x.toInt(), bounds.bottom - origin.y.toInt()) },
            fold?.orientation == FoldingFeature.Orientation.HORIZONTAL)
        val safeModifier = with(density) { Modifier.offset(region.left.toDp(), region.top.toDp())
            .size((region.right - region.left).toDp(), (region.bottom - region.top).toDp()) }
        val twoPanes = with(density) { supportsTwoPanes((region.right - region.left).toDp().value.toInt(),
            (region.bottom - region.top).toDp().value.toInt()) }
        Box(safeModifier) {
            CompositionLocalProvider(LocalTwoPaneAvailable provides twoPanes,
                LocalWorkspaceWidth provides with(density) { (region.right - region.left).toDp().value.toInt() },
                LocalWorkspaceHeight provides with(density) { (region.bottom - region.top).toDp().value.toInt() }) { content() }
        }
    }
}
