package dev.akko.miaominal

import android.Manifest
import android.content.Context
import android.content.res.Configuration
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.os.LocaleListCompat
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import dev.akko.miaominal.ui.model.AppearanceOperations
import dev.akko.miaominal.ui.model.LocalAppearanceOperations
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.tooling.preview.Preview
import androidx.core.content.ContextCompat
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.data.MiaominalRepository
import dev.akko.miaominal.data.RepositoryStatus
import dev.akko.miaominal.ui.MiaominalApp
import dev.akko.miaominal.ui.UiStrings
import dev.akko.miaominal.ui.theme.MiaominalTheme
import dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED
import dev.akko.miaominal.ui.theme.ThemeMode

private const val APPEARANCE_PREFERENCES = "appearance"
private const val THEME_MODE_KEY = "theme_mode"
private const val DYNAMIC_COLOR_KEY = "dynamic_color"
private const val SEED_COLOR_KEY = "seed_color"

private const val PERMISSIONS_PREFERENCES = "permissions"
private const val NOTIFICATIONS_ASKED_KEY = "notifications_asked"

class MainActivity : AppCompatActivity() {
    private val notificationPermissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        UiStrings.updateConfiguration(this)
        enableEdgeToEdge()
        requestNotificationPermissionOnce()
        setContent {
            MiaominalRoot()
        }
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        // Compose observes the updated configuration. Refresh callback strings
        // too, without replacing the Activity or its remembered UI state.
        UiStrings.updateConfiguration(this)
    }

    /**
     * Background keep-alive is on by default, so the foreground-service
     * notification needs POST_NOTIFICATIONS on API 33+. Ask once per install;
     * afterwards the permission is only revisited from the settings screen or
     * the system settings.
     */
    private fun requestNotificationPermissionOnce() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val container = (application as? MiaominalApplication)?.container ?: return
        if (!container.repository.devicePreferences.value.backgroundKeepAlive) return
        val granted = ContextCompat.checkSelfPermission(
            this,
            Manifest.permission.POST_NOTIFICATIONS
        ) == PackageManager.PERMISSION_GRANTED
        if (granted) return
        val preferences = getSharedPreferences(PERMISSIONS_PREFERENCES, Context.MODE_PRIVATE)
        if (preferences.getBoolean(NOTIFICATIONS_ASKED_KEY, false)) return
        preferences.edit().putBoolean(NOTIFICATIONS_ASKED_KEY, true).apply()
        notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
    }
}

@Composable
private fun MiaominalRoot() {
    val context = LocalContext.current
    val repository: MiaominalRepository = remember(context) {
        (context.applicationContext as? MiaominalApplication)?.container?.repository
            ?: InMemoryMiaominalRepository()
    }
    val agentChat = remember(context) {
        (context.applicationContext as? MiaominalApplication)?.container?.agentRepository
    }
    val preferences = remember(context) {
        context.getSharedPreferences(APPEARANCE_PREFERENCES, Context.MODE_PRIVATE)
    }
    val systemDarkTheme = isSystemInDarkTheme()
    val appearance: AppearanceOperations = viewModel(factory = viewModelFactory { initializer { AppearanceOperations(preferences) } })
    val themeMode = appearance.mode
    val dynamicColor = appearance.dynamic
    val seedColor = appearance.seed
    val darkTheme = when (themeMode) {
        ThemeMode.System -> systemDarkTheme
        ThemeMode.Light -> false
        ThemeMode.Dark -> true
    }

    // The Rust terminal palette follows the settings theme, so the effective
    // light/dark choice and the seed color are pushed to the backend once it is
    // ready and again whenever the appearance settings change.
    val repositoryStatus by repository.status.collectAsState()
    val businessSettings by repository.settings.collectAsState()
    LaunchedEffect(repositoryStatus, businessSettings.language) {
        if (repositoryStatus != RepositoryStatus.Ready) return@LaunchedEffect
        val tag = if (businessSettings.language == "English") "en" else "zh-CN"
        if (AppCompatDelegate.getApplicationLocales().toLanguageTags() != tag) {
            AppCompatDelegate.setApplicationLocales(LocaleListCompat.forLanguageTags(tag))
        }
    }
    LaunchedEffect(repositoryStatus, darkTheme, seedColor, appearance.busy) {
        if (repositoryStatus != RepositoryStatus.Ready) return@LaunchedEffect
        if (appearance.busy || appearance.error != null) return@LaunchedEffect
        val themeId = if (darkTheme) "dark" else "light"
        val seed = String.format("#%06X", seedColor and 0xFFFFFF)
        if (businessSettings.themeId != themeId || businessSettings.seedColor != seed) {
            appearance.update(repository, systemDarkTheme)
        }
    }

    CompositionLocalProvider(
        LocalAppearanceOperations provides appearance,
        dev.akko.miaominal.ui.LocalAgentConfigRepository provides
            (context.applicationContext as? MiaominalApplication)?.container?.agentConfigs
    ) {
    MiaominalTheme(
        darkTheme = darkTheme,
        dynamicColor = dynamicColor,
        seedColor = seedColor
    ) {
        MiaominalApp(
            repository = repository,
            themeMode = themeMode,
            darkTheme = darkTheme,
            dynamicColor = dynamicColor,
            seedColor = seedColor,
            onThemeModeChange = { appearance.update(repository, systemDarkTheme, nextMode = it) },
            onDynamicColorChange = { appearance.update(repository, systemDarkTheme, nextDynamic = it) },
            onSeedColorChange = { appearance.update(repository, systemDarkTheme, nextSeed = it) },
            agentChat = agentChat
        )
    }
    }
}

@Preview(showBackground = true, widthDp = 412, heightDp = 915)
@Composable
private fun MiaominalAppPreview() {
    MiaominalTheme {
        MiaominalApp(
            repository = InMemoryMiaominalRepository(),
            themeMode = ThemeMode.Light,
            darkTheme = false,
            dynamicColor = false,
            seedColor = DEFAULT_THEME_SEED,
            onThemeModeChange = {},
            onDynamicColorChange = {},
            onSeedColorChange = {}
        )
    }
}
