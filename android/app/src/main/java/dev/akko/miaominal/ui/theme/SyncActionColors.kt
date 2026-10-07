package dev.akko.miaominal.ui.theme

import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ColorScheme

/** An opaque secondary action on the sync overview's colored status container. */
internal fun syncDownloadButtonColors(scheme: ColorScheme) = ButtonColors(
    containerColor = scheme.surfaceContainerLowest,
    contentColor = scheme.onSurface,
    disabledContainerColor = scheme.surfaceContainerHigh,
    disabledContentColor = scheme.onSurfaceVariant
)
