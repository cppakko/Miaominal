package dev.akko.miaominal.ui.theme

import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.unit.dp

object UiSpacing {
    val xxs = 4.dp
    val xs = 8.dp
    val sm = 16.dp
    val md = 24.dp
    val lg = 32.dp
}

object ExpressiveMotion {
    fun <T> stateChange() = spring<T>(
        dampingRatio = Spring.DampingRatioNoBouncy,
        stiffness = Spring.StiffnessMedium
    )
}

val successColor: Color
    @Composable get() = if (MaterialTheme.colorScheme.surface.luminance() > 0.5f) {
        Color(0xFF146C43)
    } else Color(0xFF79D9A4)
