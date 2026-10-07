package dev.akko.miaominal.ui.components

import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.runtime.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

class BackGestureState {
    var progress by mutableFloatStateOf(0f)
}
val LocalBackGesture = staticCompositionLocalOf { BackGestureState() }

internal suspend fun completeBackGesture(events: Flow<Float>, gesture: BackGestureState, onBack: () -> Unit) {
    try {
        events.collect { gesture.progress = it }
        gesture.progress = 0f
        onBack()
    } catch (_: CancellationException) {
        // Cancellation is a preview only, with no navigation mutation.
    } finally { gesture.progress = 0f }
}

/** Navigation commits only once the system's back gesture completes. */
@Composable
fun WorkspaceBackHandler(enabled: Boolean = true, onBack: () -> Unit) {
    val currentBack by rememberUpdatedState(onBack)
    val gesture = LocalBackGesture.current
    PredictiveBackHandler(enabled) { events ->
        completeBackGesture(events.map { it.progress }, gesture) { currentBack() }
    }
}
