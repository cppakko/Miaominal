package dev.akko.miaominal.ui.theme

import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

// Use the Material 3 type and shape scales. Terminal rendering keeps its own
// font preferences, independent of the surrounding application UI.
private val baseline = Typography()
val Typography = Typography(
    headlineSmall = baseline.headlineSmall.copy(fontWeight = FontWeight.SemiBold),
    titleMedium = baseline.titleMedium.copy(fontWeight = FontWeight.SemiBold),
    labelLarge = baseline.labelLarge.copy(fontWeight = FontWeight.SemiBold)
)
val Shapes = Shapes(medium = RoundedCornerShape(20.dp), large = RoundedCornerShape(20.dp), extraLarge = RoundedCornerShape(28.dp))