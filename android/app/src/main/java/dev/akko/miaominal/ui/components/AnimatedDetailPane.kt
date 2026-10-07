package dev.akko.miaominal.ui.components

import androidx.compose.animation.AnimatedContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import dev.akko.miaominal.ui.theme.MiaominalMotion

/** Destination changes animate; resizing only rearranges the saved list/detail slots. */
@Composable
fun <T : Any> AnimatedDetailPane(
    selected: T?,
    twoPanes: Boolean,
    defaultDetail: T,
    listContent: @Composable () -> Unit,
    detailContent: @Composable (T) -> Unit,
    modifier: Modifier = Modifier
) {
    // A layout change must not retain an outgoing list while the wide layout
    // also composes that same saved list. The caller owns both state holders.
    key(twoPanes) {
        Row(modifier.fillMaxSize()) {
            if (twoPanes) {
                Box(Modifier.weight(1f).fillMaxSize()) { listContent() }
                VerticalDivider()
            }
            AnimatedContent(
                targetState = selected ?: if (twoPanes) defaultDetail else null,
                transitionSpec = { MiaominalMotion.surfaceTransition().using(null) },
                label = "detail-destination",
                modifier = Modifier.weight(1f).fillMaxSize().clipToBounds()
            ) { destination ->
                if (destination == null) listContent() else detailContent(destination)
            }
        }
    }
}
