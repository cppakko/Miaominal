package dev.akko.miaominal

import android.accessibilityservice.AccessibilityService
import android.content.Context
import android.os.Environment
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.ui.components.StartupStoragePermission
import dev.akko.miaominal.ui.theme.MiaominalTheme
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class StoragePermissionUiTest {
    @get:Rule val compose = createComposeRule()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val preferences = context.getSharedPreferences("permissions", Context.MODE_PRIVATE)
    private var askedBefore: Boolean? = null

    @Before fun setUp() {
        org.junit.Assert.assertFalse("Run the permission prompt test without all files access", Environment.isExternalStorageManager())
        askedBefore = if (preferences.contains("storage_asked")) preferences.getBoolean("storage_asked", false) else null
        preferences.edit().remove("storage_asked").commit()
    }

    @After fun tearDown() {
        val edit = preferences.edit()
        askedBefore?.let { edit.putBoolean("storage_asked", it) } ?: edit.remove("storage_asked")
        edit.commit()
    }

    @Test fun startupWaitsForNotificationsAndOpensSystemSettingsOnlyOnce() {
        val enabled = mutableStateOf(false)
        val restoration = StateRestorationTester(compose)
        restoration.setContent { MiaominalTheme { StartupStoragePermission(enabled.value) } }
        val title = context.getString(R.string.sftp_storage_permission_title)
        compose.onNodeWithText(title).assertDoesNotExist()
        compose.runOnIdle { enabled.value = true }
        compose.onNodeWithText(title).assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.sftp_storage_allow)).performClick()
        compose.waitUntil(10_000) {
            instrumentation.uiAutomation.rootInActiveWindow?.packageName?.toString() == "com.android.settings"
        }
        instrumentation.uiAutomation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
        compose.waitForIdle()
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithText(title).assertDoesNotExist()
    }

}
