package dev.akko.miaominal.ui.theme

import android.app.UiModeManager
import android.content.Context
import android.os.Build
import androidx.compose.runtime.*
import kotlinx.coroutines.*

enum class UiContrast { System, Standard, Medium, High }

fun defaultUiContrast(apiLevel: Int) = if (apiLevel >= 34) UiContrast.System else UiContrast.Standard
fun parseUiContrast(value: String?, apiLevel: Int): UiContrast =
    value?.let { runCatching { UiContrast.valueOf(it) }.getOrNull() } ?: defaultUiContrast(apiLevel)
fun terminalFontScale(followSystem: Boolean, systemScale: Float) = if (followSystem) systemScale else 1f

class UiPreferences(context: Context) {
    private val store = context.applicationContext.getSharedPreferences("ui_preferences", Context.MODE_PRIVATE)
    var contrast by mutableStateOf(parseUiContrast(store.getString("contrast", null), Build.VERSION.SDK_INT))
        private set
    var followTerminalFontScale by mutableStateOf(store.getBoolean("terminal_follow_font_scale", false))
        private set
    var systemContrast by mutableFloatStateOf(0f)
    var busy by mutableStateOf(false); private set
    var saveFailed by mutableStateOf(false); private set
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    fun updateContrast(value: UiContrast) {
        if (busy) return
        val previous = contrast
        contrast = value; busy = true; saveFailed = false
        scope.launch {
            try {
                if (!withContext(Dispatchers.IO) { store.edit().putString("contrast", value.name).commit() }) {
                    contrast = previous; saveFailed = true
                    withContext(Dispatchers.IO) { store.edit().putString("contrast", previous.name).commit() }
                }
            } catch (_: Exception) { contrast = previous; saveFailed = true }
            finally { busy = false }
        }
    }
    fun updateFollowTerminalFontScale(value: Boolean) {
        if (busy) return
        val previous = followTerminalFontScale
        followTerminalFontScale = value; busy = true; saveFailed = false
        scope.launch {
            try {
                if (!withContext(Dispatchers.IO) { store.edit().putBoolean("terminal_follow_font_scale", value).commit() }) {
                    followTerminalFontScale = previous; saveFailed = true
                    withContext(Dispatchers.IO) { store.edit().putBoolean("terminal_follow_font_scale", previous).commit() }
                }
            } catch (_: Exception) { followTerminalFontScale = previous; saveFailed = true }
            finally { busy = false }
        }
    }
    val contrastLevel: Double get() = when (contrast) {
        UiContrast.System -> systemContrast.toDouble().coerceIn(0.0, 1.0)
        UiContrast.Standard -> 0.0
        UiContrast.Medium -> 0.5
        UiContrast.High -> 1.0
    }
}

val LocalUiPreferences = staticCompositionLocalOf<UiPreferences?> { null }

@Composable
fun rememberUiPreferences(context: Context): UiPreferences {
    val preferences = remember(context.applicationContext) { UiPreferences(context) }
    DisposableEffect(context.applicationContext) {
        if (Build.VERSION.SDK_INT >= 34) {
            val manager = context.getSystemService(UiModeManager::class.java)
            preferences.systemContrast = manager.contrast
            val listener = UiModeManager.ContrastChangeListener { preferences.systemContrast = it }
            manager.addContrastChangeListener(context.mainExecutor, listener)
            onDispose { manager.removeContrastChangeListener(listener) }
        } else onDispose { }
    }
    return preferences
}
