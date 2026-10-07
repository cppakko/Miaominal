package dev.akko.miaominal.ui.model

import android.content.SharedPreferences
import androidx.compose.runtime.*
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.akko.miaominal.data.MiaominalRepository
import dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED
import dev.akko.miaominal.ui.theme.ThemeMode
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Confirm backend theme changes before publishing the new device-local appearance. */
class AppearanceOperations(private val preferences: SharedPreferences) : ViewModel() {
    var mode by mutableStateOf(ThemeMode.fromStoredValue(preferences.getString("theme_mode", null))); private set
    var dynamic by mutableStateOf(preferences.getBoolean("dynamic_color", true)); private set
    var seed by mutableIntStateOf(preferences.getInt("seed_color", DEFAULT_THEME_SEED)); private set
    var busy by mutableStateOf(false); private set
    var error by mutableStateOf<Throwable?>(null); private set

    fun update(repository: MiaominalRepository, systemDark: Boolean, nextMode: ThemeMode = mode, nextDynamic: Boolean = dynamic, nextSeed: Int = seed) {
        if (busy) return
        busy = true; error = null
        val oldMode = mode; val oldDynamic = dynamic; val oldSeed = seed
        val backendBefore = repository.settings.value
        viewModelScope.launch {
            val targetDark = when (nextMode) { ThemeMode.System -> systemDark; ThemeMode.Light -> false; ThemeMode.Dark -> true }
            val targetId = if (targetDark) "dark" else "light"
            val targetSeed = String.format("#%06X", nextSeed and 0xFFFFFF)
            var backendChanged = false
            try {
                if (backendBefore.themeId != targetId || backendBefore.seedColor != targetSeed) {
                    repository.updateSettings { it.copy(themeId = targetId, seedColor = targetSeed) }.getOrThrow()
                    backendChanged = true
                }
                val persisted = withContext(Dispatchers.IO) { preferences.edit()
                    .putString("theme_mode", nextMode.name).putBoolean("dynamic_color", nextDynamic).putInt("seed_color", nextSeed).commit() }
                if (!persisted) {
                    // SharedPreferences updates its memory even when disk commit fails.
                    withContext(Dispatchers.IO) { preferences.edit().putString("theme_mode", oldMode.name)
                        .putBoolean("dynamic_color", oldDynamic).putInt("seed_color", oldSeed).commit() }
                    if (backendChanged) repository.updateSettings { it.copy(themeId = backendBefore.themeId, seedColor = backendBefore.seedColor) }
                    throw java.io.IOException("Appearance preferences could not be saved")
                }
                mode = nextMode; dynamic = nextDynamic; seed = nextSeed
            } catch (failure: Exception) { error = failure }
            finally { busy = false }
        }
    }
}

val LocalAppearanceOperations = staticCompositionLocalOf<AppearanceOperations?> { null }
