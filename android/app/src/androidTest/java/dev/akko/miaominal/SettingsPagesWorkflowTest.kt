package dev.akko.miaominal

import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.*
import dev.akko.miaominal.ui.LocalMiaominalRepository
import dev.akko.miaominal.ui.model.SettingsOperations
import dev.akko.miaominal.ui.screens.*
import dev.akko.miaominal.ui.theme.MiaominalTheme
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Compile by default. Execution needs the user's designated-device/reinstallation consent. */
class SettingsPagesWorkflowTest {
    @get:Rule val compose = createComposeRule()
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val operations = SettingsOperations()
    private var wide by mutableStateOf(false)
    private fun label(id: Int) = context.getString(id)
    private fun showVault(repository: MiaominalRepository) {
        compose.setContent { CompositionLocalProvider(LocalMiaominalRepository provides repository) {
            MiaominalTheme { Box(Modifier.requiredSize(if (wide) 840.dp else 320.dp, 640.dp)) {
                MobileVaultSettingsPage(operations, Modifier.fillMaxSize(), null)
            } }
        } }
    }

    @Test fun failedEnableRetainsFieldsAndCancelRequiresDiscard() {
        val base = InMemoryMiaominalRepository()
        val repository = object : MiaominalRepository by base {
            override suspend fun vaultEnable(passphrase: String): Result<Unit> = Result.failure(IllegalStateException("save failed"))
        }
        showVault(repository)
        compose.onNodeWithText(label(R.string.desktop_vault_enable)).performClick()
        compose.onNodeWithText(label(R.string.settings_new_password)).performTextInput("same-pass")
        compose.onNodeWithText(label(R.string.desktop_vault_confirm_passphrase)).performTextInput("same-pass")
        compose.onAllNodesWithText(label(R.string.desktop_vault_enable)).filter(hasClickAction()).onLast().performClick()
        compose.onNodeWithText("save failed").assertIsDisplayed()
        compose.runOnIdle { assertEquals("same-pass", operations.password); wide = true }
        compose.runOnIdle { assertEquals("same-pass", operations.confirmation) }
        compose.onNodeWithContentDescription(label(R.string.action_back)).performClick()
        compose.onNodeWithText(label(R.string.settings_discard_title)).assertIsDisplayed()
        compose.onNodeWithText(label(R.string.tool_action_cancel)).performClick()
        compose.runOnIdle { assertNotNull(operations.vaultForm); assertTrue(operations.passwordDirty) }
        compose.onNodeWithContentDescription(label(R.string.action_back)).performClick()
        compose.onNodeWithText(label(R.string.settings_discard)).performClick()
        compose.runOnIdle { assertFalse(operations.passwordDirty); assertNull(operations.vaultForm) }
    }

    @Test fun disableFailureStaysVisibleWithNoPasswordPanel() {
        val base = InMemoryMiaominalRepository(vault = VaultStatus(VaultMode.Unlocked, true))
        val repository = object : MiaominalRepository by base {
            override suspend fun vaultDisable(): Result<Unit> = Result.failure(IllegalStateException("migration failed"))
        }
        showVault(repository)
        compose.onNodeWithTag("vault-settings-list").performScrollToNode(hasText(label(R.string.desktop_vault_disable)))
        compose.onNodeWithText(label(R.string.desktop_vault_disable)).performClick()
        compose.onNodeWithText(label(R.string.settings_disable_vault_explanation)).assertIsDisplayed()
        compose.onAllNodesWithText(label(R.string.desktop_vault_disable)).filter(hasClickAction()).onLast().performClick()
        compose.onNodeWithTag("vault-settings-list").performScrollToNode(hasText("migration failed"))
        compose.onNodeWithText("migration failed").assertIsDisplayed()
        compose.runOnIdle { assertNull(operations.vaultForm) }
    }

    @Test fun previewShowsEveryEntryAndWarningAndSearchDoesNotChangeImportScope() {
        val base = InMemoryMiaominalRepository()
        val entry = ImportEntryModel("Host", "group", "example.test", 22, "user", "password", "", "", true, false, false, "")
        val preview = ImportPreviewModel(123, "OpenSSH", (0..24).map { entry.copy(name = "Host $it") },
            (0..15).map { ImportWarningModel("warning", "Host $it", "Warning $it") })
        base.seedImportPreview(preview)
        compose.setContent { CompositionLocalProvider(LocalMiaominalRepository provides base) { MiaominalTheme { SettingsImportFlow(operations) } } }
        compose.runOnIdle { operations.openImport(ImportSourceModel.OpenSsh); operations.loadImport(base) { Result.success(preview) } }
        compose.onNodeWithTag("settings-import-preview").performScrollToNode(hasText("Host 24"))
        compose.onNodeWithText("Host 24").assertIsDisplayed()
        compose.onNodeWithTag("settings-import-preview").performScrollToNode(hasText("Host 15: Warning 15"))
        compose.onNodeWithText("Host 15: Warning 15").assertIsDisplayed()
        compose.onNodeWithTag("settings-import-preview").performScrollToNode(hasText(label(R.string.settings_preview_search)))
        compose.onNodeWithText(label(R.string.settings_preview_search)).performTextInput("Host 24")
        compose.onNodeWithText(context.getString(R.string.settings_confirm_batch, 25)).performClick()
        compose.onNodeWithText(label(R.string.settings_import_all)).assertIsDisplayed()
        compose.onNodeWithText(label(R.string.keychain_source_imported)).performClick()
        compose.waitUntil { operations.importedCount == 25 }
        compose.runOnIdle { assertEquals(25, base.hosts.value.size) }
    }
}
