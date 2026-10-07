package dev.akko.miaominal.ui.components

import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.painter.Painter

/** Decorative icons stay silent; action icons expose their existing name as a tooltip. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun IconHint(label: String?, content: @Composable () -> Unit) {
    if (label.isNullOrBlank()) content() else TooltipBox(
        positionProvider = TooltipDefaults.rememberPlainTooltipPositionProvider(),
        tooltip = { PlainTooltip { Text(label) } }, state = rememberTooltipState(), content = content)
}

@Composable
fun HintIcon(imageVector: ImageVector, contentDescription: String?, modifier: Modifier = Modifier,
    tint: Color = LocalContentColor.current) = IconHint(contentDescription) {
    Icon(imageVector, contentDescription, modifier, tint)
}

@Composable
fun HintIcon(painter: Painter, contentDescription: String?, modifier: Modifier = Modifier,
    tint: Color = LocalContentColor.current) = IconHint(contentDescription) {
    Icon(painter, contentDescription, modifier, tint)
}
