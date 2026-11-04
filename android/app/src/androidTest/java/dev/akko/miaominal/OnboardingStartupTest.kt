package dev.akko.miaominal

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.AndroidPlatformHost
import dev.akko.miaominal.data.BusinessSettings
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.data.MiaominalRepository
import dev.akko.miaominal.data.NativeBackendClient
import dev.akko.miaominal.data.NativeMiaominalRepository
import dev.akko.miaominal.data.RepositoryStatus
import dev.akko.miaominal.ui.MiaominalApp
import dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED
import dev.akko.miaominal.ui.theme.MiaominalTheme
import dev.akko.miaominal.ui.theme.ThemeMode
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class OnboardingStartupTest {
    @get:Rule val compose = createComposeRule()

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val backing = InMemoryMiaominalRepository(settings = BusinessSettings())
    private val status = MutableStateFlow<RepositoryStatus>(RepositoryStatus.Initializing)
    private val repository = object : MiaominalRepository by backing {
        override val status = this@OnboardingStartupTest.status
    }
    private val onboardingTitle get() = context.getString(R.string.onboarding_step1_title)

    private fun setContent() {
        compose.setContent {
            MiaominalTheme {
                MiaominalApp(
                    repository = repository,
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
    }

    @Test
    fun returningUserNeverSeesOnboardingWhileSettingsLoad() {
        setContent()
        compose.onNodeWithText(onboardingTitle).assertDoesNotExist()

        runBlocking { backing.updateSettings { it.copy(onboardingCompleted = true) }.getOrThrow() }
        compose.onNodeWithText(onboardingTitle).assertDoesNotExist()

        compose.runOnIdle { status.value = RepositoryStatus.Ready }
        compose.onNodeWithText(onboardingTitle).assertDoesNotExist()
        compose.onNodeWithTag("main-navigation-bar").assertIsDisplayed()
    }

    @Test
    fun firstLaunchShowsOnboardingOnlyAfterSettingsLoad() {
        setContent()
        compose.onNodeWithText(onboardingTitle).assertDoesNotExist()

        compose.runOnIdle { status.value = RepositoryStatus.Ready }
        compose.onNodeWithText(onboardingTitle).assertIsDisplayed()

        runBlocking { backing.updateSettings { it.copy(onboardingCompleted = true) }.getOrThrow() }
        compose.onNodeWithText(onboardingTitle).assertDoesNotExist()
        compose.onNodeWithTag("main-navigation-bar").assertIsDisplayed()

        // The About page can still reopen the guide after startup.
        runBlocking { backing.updateSettings { it.copy(onboardingCompleted = false) }.getOrThrow() }
        compose.onNodeWithText(onboardingTitle).assertIsDisplayed()
    }

    @Test
    fun startupFailureDoesNotShowOnboardingFromDefaultSettings() {
        setContent()
        compose.runOnIdle { status.value = RepositoryStatus.Failed("backend unavailable") }
        compose.onNodeWithText(onboardingTitle).assertDoesNotExist()
    }

    @Test
    fun nativeRepositoryBecomesReadyOnlyAfterSnapshotsLoad() = runBlocking<Unit> {
        val client = NativeBackendClient.get(context)
        withTimeout(30_000) { client.ensureStarted() }
        val native = NativeMiaominalRepository(context, client, AndroidPlatformHost(context).credentialStore)

        native.ensureStarted().getOrThrow()
        assertEquals(RepositoryStatus.Initializing, native.status.value)

        native.refreshAll().getOrThrow()
        assertEquals(RepositoryStatus.Ready, native.status.value)
        assertEquals(
            client.currentBackend()!!.settingsSnapshot().onboardingCompleted,
            native.settings.value.onboardingCompleted
        )
    }
}
