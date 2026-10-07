package dev.akko.miaominal.ui.components

import android.graphics.Typeface
import android.graphics.fonts.FontFamily as AndroidFontFamily
import android.graphics.fonts.SystemFonts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Remove
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.akko.miaominal.R
import dev.akko.miaominal.ui.fonts.fontFamilyNameFromFile
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlin.math.round

@Composable
internal fun FontMetricControl(
    value: Float,
    label: String,
    range: ClosedFloatingPointRange<Float>,
    onCommit: (Float) -> Unit,
    onDraftChange: (Float) -> Unit = {}
) {
    var draft by remember(value) { mutableFloatStateOf(value) }
    val decrease = stringResource(R.string.font_decrease, label)
    val increase = stringResource(R.string.font_increase, label)
    Column {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("$label: $draft", modifier = Modifier.weight(1f), style = MaterialTheme.typography.titleSmall)
            IconButton(enabled = draft > range.start, onClick = {
                draft = (draft - 0.5f).coerceIn(range); onDraftChange(draft); onCommit(draft)
            }) { Icon(Icons.Outlined.Remove, decrease) }
            IconButton(enabled = draft < range.endInclusive, onClick = {
                draft = (draft + 0.5f).coerceIn(range); onDraftChange(draft); onCommit(draft)
            }) { Icon(Icons.Outlined.Add, increase) }
        }
        Slider(
            value = draft,
            onValueChange = { draft = (round(it * 2) / 2).coerceIn(range); onDraftChange(draft) },
            onValueChangeFinished = { onCommit(draft) },
            valueRange = range,
            modifier = Modifier.semantics { contentDescription = label }
        )
    }
}

@Composable
internal fun TerminalFontPreview(primary: String, fallback: String, size: Float, lineHeight: Float) {
    val typeface by produceState(Typeface.MONOSPACE, primary, fallback) {
        value = withContext(Dispatchers.Default) {
            runCatching {
                val fonts = SystemFonts.getAvailableFonts()
                fun family(name: String) = fonts.firstOrNull {
                    it.file?.name?.let(::fontFamilyNameFromFile) == name
                }?.let { AndroidFontFamily.Builder(it).build() }
                val main = family(primary)
                if (main == null) Typeface.MONOSPACE
                else Typeface.CustomFallbackBuilder(main).apply {
                    family(fallback)?.let { addCustomFallback(it) }
                    setSystemFallback("monospace")
                }.build()
            }.getOrDefault(Typeface.MONOSPACE)
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.font_preview), style = MaterialTheme.typography.labelLarge)
        Surface(color = MaterialTheme.colorScheme.surfaceContainerHighest, shape = MaterialTheme.shapes.medium) {
            Text(
                text = "user@host:~$ ls -la\nHello, 世界 · 日本語 · 한국어\n0123456789  {} [] <> / | ~",
                style = MaterialTheme.typography.bodyMedium.copy(
                    fontFamily = FontFamily(typeface), fontSize = size.sp, lineHeight = lineHeight.sp
                ),
                modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(12.dp)
            )
        }
    }
}
