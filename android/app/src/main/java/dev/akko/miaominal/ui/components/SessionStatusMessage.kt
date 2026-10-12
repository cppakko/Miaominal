package dev.akko.miaominal.ui.components

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.*
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.ui.theme.MiaominalMotion
import dev.akko.miaominal.ui.theme.UiSpacing

/** Shared status presentation for the connection overlay and the file browser. */
@Composable
internal fun SessionStatusMessage(status: String, busy: Boolean, modifier: Modifier = Modifier,
    failed: Boolean = false) {
    val colors = MaterialTheme.colorScheme
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        // Reserve the indicator's space when a prompt or result replaces loading.
        Box(Modifier.size(32.dp).clearAndSetSemantics {}, contentAlignment = Alignment.Center) {
            if (busy) CircularProgressIndicator(
                Modifier.fillMaxSize(), color = colors.primary,
                trackColor = colors.surfaceContainerHighest, strokeWidth = 3.dp
            )
        }
        Spacer(Modifier.height(UiSpacing.sm))
        AnimatedContent(
            targetState = status,
            transitionSpec = {
                fadeIn(MiaominalMotion.fadeThroughIn()) togetherWith
                    fadeOut(MiaominalMotion.fadeThroughOut())
            },
            label = "connection-status",
            modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite }
        ) { message ->
            Text(message, style = MaterialTheme.typography.bodyLarge,
                color = if (failed) colors.error else colors.onSurfaceVariant,
                textAlign = TextAlign.Center)
        }
    }
}
