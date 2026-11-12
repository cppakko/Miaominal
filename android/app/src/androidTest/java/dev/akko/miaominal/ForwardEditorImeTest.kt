package dev.akko.miaominal

import android.content.Context
import android.Manifest
import android.provider.Settings
import android.view.WindowManager
import android.view.inputmethod.InputMethodManager
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import androidx.annotation.StringRes
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.unit.dp
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.BusinessSettings
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.ui.MiaominalApp
import dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED
import dev.akko.miaominal.ui.theme.MiaominalTheme
import dev.akko.miaominal.ui.theme.ThemeMode
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class ForwardEditorImeTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()
    private var previousKeyboardPreference: String? = null
    private var keyboardPreferenceCaptured = false

    @Before
    fun openEditor() {
        previousKeyboardPreference = Settings.Secure.getString(
            compose.activity.contentResolver, "show_ime_with_hard_keyboard"
        )
        keyboardPreferenceCaptured = true
        setKeyboardPreference("1")
        compose.runOnUiThread {
            compose.activity.enableEdgeToEdge()
            compose.activity.window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
        }
        val repository = InMemoryMiaominalRepository(
            hosts = TestFixtures.hosts(),
            settings = BusinessSettings(onboardingCompleted = true)
        )
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
        compose.onNodeWithText(string(R.string.nav_tools)).performClick()
        compose.onNodeWithText(string(R.string.tool_forward)).performClick()
        compose.onNodeWithContentDescription(string(R.string.forward_add_rule)).performClick()
    }

    @After
    fun hideKeyboard() {
        try {
            compose.runOnUiThread {
                inputMethodManager().hideSoftInputFromWindow(compose.activity.window.decorView.windowToken, 0)
            }
        } finally {
            if (keyboardPreferenceCaptured) setKeyboardPreference(previousKeyboardPreference)
        }
    }

    @Test
    fun listenPortStaysAboveTheKeyboardWhileTyping() {
        assertPortStaysVisible(R.string.forward_source_port, "15432")
    }

    @Test
    fun targetPortStaysAboveTheKeyboardWhileTyping() {
        assertPortStaysVisible(R.string.forward_destination_port, "5432")
    }

    private fun assertPortStaysVisible(@StringRes label: Int, port: String) {
        val field = compose.onNode(hasSetTextAction() and hasText(string(label)))
        val minimumKeyboardHeight = with(compose.density) { 100.dp.toPx() }
        // Tap the field where the user sees it, without pre-scrolling it to
        // the top (which would conceal a missing IME viewport adjustment).
        field.performClick()
        compose.runOnUiThread {
            // Show the real IME. The hardware-keyboard preference is restored
            // by tear-down, including when an assertion fails.
            @Suppress("DEPRECATION")
            inputMethodManager().showSoftInput(compose.activity.currentFocus, InputMethodManager.SHOW_FORCED)
        }
        compose.waitUntil(timeoutMillis = 10_000) {
            val insets = ViewCompat.getRootWindowInsets(compose.activity.window.decorView)
            insets?.isVisible(WindowInsetsCompat.Type.ime()) == true &&
                insets.getInsets(WindowInsetsCompat.Type.ime()).bottom > minimumKeyboardHeight
        }
        compose.waitForIdle()
        field.performTextInput(port)
        field.assertTextContains(port)

        val fieldBottom = with(compose.density) { field.getUnclippedBoundsInRoot().bottom.toPx() }
        compose.runOnUiThread {
            val decor = compose.activity.window.decorView
            val keyboardHeight = ViewCompat.getRootWindowInsets(decor)!!
                .getInsets(WindowInsetsCompat.Type.ime()).bottom
            val keyboardTop = decor.height - keyboardHeight
            assertTrue("The full keyboard must remain open while typing, not only its hardware-keyboard toolbar",
                keyboardHeight > minimumKeyboardHeight)
            assertTrue("The entire port field must stay above the IME: field bottom=$fieldBottom, IME top=$keyboardTop",
                fieldBottom <= keyboardTop + 1f)
        }
    }

    private fun string(@StringRes id: Int) = compose.activity.getString(id)

    private fun inputMethodManager() =
        compose.activity.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager

    private fun setKeyboardPreference(value: String?) {
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        automation.adoptShellPermissionIdentity(Manifest.permission.WRITE_SECURE_SETTINGS)
        try {
            check(Settings.Secure.putString(compose.activity.contentResolver, "show_ime_with_hard_keyboard", value))
        } finally {
            automation.dropShellPermissionIdentity()
        }
    }
}
