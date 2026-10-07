package dev.akko.miaominal.ui.components

import android.graphics.Color as AndroidColor
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Colorize
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import dev.akko.miaominal.ui.components.HintIcon as Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import kotlin.math.roundToInt
import androidx.compose.ui.unit.dp
import dev.akko.miaominal.R

@Composable
fun SeedColorPicker(
    seedColor: Int,
    onSeedColorChange: (Int) -> Unit,
    modifier: Modifier = Modifier
) {
    var dialogVisible by rememberSaveable { mutableStateOf(false) }
    val color = Color(seedColor)

    ListItem(
        headlineContent = { Text(stringResource(R.string.appearance_manual_color)) },
        supportingContent = { Text(color.toHex()) },
        leadingContent = {
            Surface(
                modifier = Modifier.size(40.dp),
                shape = CircleShape,
                color = color,
                border = androidx.compose.foundation.BorderStroke(
                    width = 1.dp,
                    color = MaterialTheme.colorScheme.outline
                ),
                content = {}
            )
        },
        trailingContent = {
            IconButton(onClick = { dialogVisible = true }) {
                Icon(
                    imageVector = Icons.Outlined.Colorize,
                    contentDescription = stringResource(R.string.appearance_choose_color)
                )
            }
        },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = modifier.clickable { dialogVisible = true }
    )

    if (dialogVisible) {
        SeedColorPickerDialog(
            initialColor = color,
            onDismissRequest = { dialogVisible = false },
            onConfirm = { selected ->
                onSeedColorChange(selected.toArgb())
                dialogVisible = false
            }
        )
    }
}

@Composable
private fun SeedColorPickerDialog(
    initialColor: Color,
    onDismissRequest: () -> Unit,
    onConfirm: (Color) -> Unit
) {
    val initialHsv = FloatArray(3).also { AndroidColor.colorToHSV(initialColor.toArgb(), it) }
    var hue by rememberSaveable(initialColor.toArgb()) { mutableFloatStateOf(initialHsv[0]) }
    var saturation by rememberSaveable(initialColor.toArgb()) {
        mutableFloatStateOf(initialHsv[1])
    }
    var brightness by rememberSaveable(initialColor.toArgb()) {
        mutableFloatStateOf(initialHsv[2])
    }
    var hexValue by rememberSaveable(initialColor.toArgb()) {
        mutableStateOf(initialColor.toHex().removePrefix("#"))
    }
    val selectedColor = Color.hsv(hue, saturation, brightness)

    fun updateHexFromHsv() {
        hexValue = Color.hsv(hue, saturation, brightness).toHex().removePrefix("#")
    }

    AlertDialog(
        onDismissRequest = onDismissRequest,
        title = { Text(stringResource(R.string.appearance_choose_color)) },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(16.dp)
            ) {
                SaturationBrightnessPanel(
                    hue = hue,
                    saturation = saturation,
                    brightness = brightness,
                    onValueChange = { nextSaturation, nextBrightness ->
                        saturation = nextSaturation
                        brightness = nextBrightness
                        updateHexFromHsv()
                    }
                )
                HueBar(
                    hue = hue,
                    onHueChange = {
                        hue = it
                        updateHexFromHsv()
                    }
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Surface(
                        modifier = Modifier.size(48.dp),
                        shape = CircleShape,
                        color = selectedColor,
                        border = androidx.compose.foundation.BorderStroke(
                            width = 1.dp,
                            color = MaterialTheme.colorScheme.outline
                        ),
                        content = {}
                    )
                    Spacer(Modifier.width(16.dp))
                    OutlinedTextField(
                        value = hexValue,
                        onValueChange = { input ->
                            val normalized = input
                                .filter { it.isDigit() || it.uppercaseChar() in 'A'..'F' }
                                .take(6)
                                .uppercase()
                            hexValue = normalized
                            if (normalized.length == 6) {
                                val argb = ("FF$normalized").toLong(16).toInt()
                                val hsv = FloatArray(3)
                                AndroidColor.colorToHSV(argb, hsv)
                                hue = hsv[0]
                                saturation = hsv[1]
                                brightness = hsv[2]
                            }
                        },
                        modifier = Modifier.weight(1f),
                        singleLine = true,
                        prefix = { Text("#") },
                        label = { Text(stringResource(R.string.appearance_hex_color)) },
                        isError = hexValue.length != 6
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = hexValue.length == 6,
                onClick = { onConfirm(selectedColor) }
            ) {
                Text(stringResource(R.string.action_confirm))
            }
        },
        dismissButton = {
            TextButton(onClick = onDismissRequest) {
                Text(stringResource(R.string.action_cancel))
            }
        }
    )
}

@Composable
private fun SaturationBrightnessPanel(
    hue: Float,
    saturation: Float,
    brightness: Float,
    onValueChange: (Float, Float) -> Unit
) {
    val currentOnValueChange by rememberUpdatedState(onValueChange)
    val panelLabel = stringResource(R.string.appearance_saturation_brightness)
    val panelValue = stringResource(
        R.string.appearance_saturation_brightness_value,
        (saturation * 100).roundToInt(), (brightness * 100).roundToInt()
    )
    val increaseSaturation = stringResource(R.string.appearance_increase_saturation)
    val decreaseSaturation = stringResource(R.string.appearance_decrease_saturation)
    val increaseBrightness = stringResource(R.string.appearance_increase_brightness)
    val decreaseBrightness = stringResource(R.string.appearance_decrease_brightness)
    Canvas(
        modifier = Modifier
            .fillMaxWidth()
            .aspectRatio(1.6f)
            .clip(MaterialTheme.shapes.medium)
            .semantics {
                contentDescription = panelLabel
                stateDescription = panelValue
                customActions = listOf(
                    CustomAccessibilityAction(increaseSaturation) {
                        currentOnValueChange((saturation + 0.05f).coerceAtMost(1f), brightness); true
                    },
                    CustomAccessibilityAction(decreaseSaturation) {
                        currentOnValueChange((saturation - 0.05f).coerceAtLeast(0f), brightness); true
                    },
                    CustomAccessibilityAction(increaseBrightness) {
                        currentOnValueChange(saturation, (brightness + 0.05f).coerceAtMost(1f)); true
                    },
                    CustomAccessibilityAction(decreaseBrightness) {
                        currentOnValueChange(saturation, (brightness - 0.05f).coerceAtLeast(0f)); true
                    }
                )
            }
            .pointerInput(hue) {
                detectTapGestures { offset ->
                    currentOnValueChange(
                        (offset.x / size.width).coerceIn(0f, 1f),
                        (1f - offset.y / size.height).coerceIn(0f, 1f)
                    )
                }
            }
            .pointerInput(hue) {
                detectDragGestures(
                    onDragStart = { offset ->
                        currentOnValueChange(
                            (offset.x / size.width).coerceIn(0f, 1f),
                            (1f - offset.y / size.height).coerceIn(0f, 1f)
                        )
                    },
                    onDrag = { change, _ ->
                        currentOnValueChange(
                            (change.position.x / size.width).coerceIn(0f, 1f),
                            (1f - change.position.y / size.height).coerceIn(0f, 1f)
                        )
                    }
                )
            }
    ) {
        drawRect(Brush.horizontalGradient(listOf(Color.White, Color.hsv(hue, 1f, 1f))))
        drawRect(Brush.verticalGradient(listOf(Color.Transparent, Color.Black)))
        val indicator = Offset(saturation * size.width, (1f - brightness) * size.height)
        drawCircle(Color.Black, radius = 10.dp.toPx(), center = indicator, style = Stroke(4.dp.toPx()))
        drawCircle(Color.White, radius = 10.dp.toPx(), center = indicator, style = Stroke(2.dp.toPx()))
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HueBar(
    hue: Float,
    onHueChange: (Float) -> Unit
) {
    val hueLabel = stringResource(R.string.appearance_hue)
    val hueValue = stringResource(R.string.appearance_hue_value, hue.roundToInt())
    Slider(
        value = hue,
        onValueChange = onHueChange,
        valueRange = 0f..360f,
        modifier = Modifier.fillMaxWidth().height(48.dp).semantics {
            contentDescription = hueLabel
            stateDescription = hueValue
        },
        track = {
            Canvas(Modifier.fillMaxWidth().height(16.dp).clip(CircleShape)) {
                drawRect(
                    Brush.horizontalGradient((0..6).map { Color.hsv(it * 60f, 1f, 1f) })
                )
            }
        }
    )
}

private fun Color.toHex(): String = "#%06X".format(toArgb() and 0xFFFFFF)
