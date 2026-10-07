package dev.akko.miaominal

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import dev.akko.miaominal.data.BusinessSettings
import dev.akko.miaominal.data.InMemoryMiaominalRepository
import dev.akko.miaominal.data.MiaominalRepository
import dev.akko.miaominal.data.SyncConfigEdit
import dev.akko.miaominal.data.SyncConflictModel
import dev.akko.miaominal.data.SyncPhaseModel
import dev.akko.miaominal.data.SyncProviderModel
import dev.akko.miaominal.data.SyncStateModel
import dev.akko.miaominal.data.SyncStatusModel
import dev.akko.miaominal.ui.MiaominalApp
import dev.akko.miaominal.ui.theme.DEFAULT_THEME_SEED
import dev.akko.miaominal.ui.theme.MiaominalTheme
import dev.akko.miaominal.ui.theme.ThemeMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/**
 * UI acceptance against the injected test repository: provider editing,
 * automatic-sync wiring, conflict resolution, and the typed status rows.
 */
class SyncUiTest {
    @get:Rule
    val compose = createComposeRule()

    private val repository = InMemoryMiaominalRepository(
        hosts = TestFixtures.hosts(),
        statuses = TestFixtures.hostStatus(),
        settings = BusinessSettings(onboardingCompleted = true)
    )
    private var injectedRepository: MiaominalRepository = repository

    private fun setContent() {
        compose.setContent {
            MiaominalTheme {
                MiaominalApp(
                    repository = injectedRepository,
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
        compose.waitForIdle()
    }

    private fun openSyncSettings() {
        compose.onNodeWithText("设置").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("同步").performClick()
        compose.waitForIdle()
    }

    @Test
    fun hostSyncButtonPushesConfiguredSyncAndShowsFeedback() {
        repository.seedSync(
            SyncStateModel(
                phase = SyncPhaseModel.Idle,
                config = repository.sync.value.config.copy(provider = SyncProviderModel.Gist)
            )
        )
        setContent()
        compose.onNodeWithContentDescription("推送本地配置").performClick()
        compose.waitUntil { repository.sync.value.status is SyncStatusModel.Pushed }
        compose.onNodeWithText("本地配置已同步到远端").assertIsDisplayed()
    }

    @Test
    fun hostSyncButtonOpensSetupWhenUnconfigured() {
        setContent()
        compose.onNodeWithContentDescription("推送本地配置").performClick()
        compose.onNodeWithText("完成同步设置").assertIsDisplayed()
        assertEquals(SyncStatusModel.Idle, repository.sync.value.status)
    }

    @Test
    fun hostSyncButtonIsDisabledDuringSync() {
        repository.seedSync(SyncStateModel(phase = SyncPhaseModel.Pushing))
        setContent()
        compose.onNodeWithContentDescription("推送本地配置").assertIsNotEnabled()
    }

    @Test
    fun syncPageEditsProviderAndEnablesAutoSync() {
        repository.seedSync(SyncStateModel(config = repository.sync.value.config.copy(hasPassphrase = true)))
        setContent()
        openSyncSettings()

        compose.onNodeWithTag("sync-edit-connection").performScrollTo().performClick()
        compose.onNodeWithText("WebDAV").performScrollTo().performClick()
        compose.waitForIdle()
        assertEquals(SyncProviderModel.None, repository.sync.value.config.provider)

        compose.onNodeWithTag("sync-webdav-url")
            .performScrollTo()
            .performTextInput("https://dav.example.com/sync.json")
        compose.onNodeWithTag("sync-credential").performScrollTo().performTextInput("app-password")
        assertEquals("", repository.sync.value.config.webdavUrl)
        compose.onNodeWithTag("sync-save-connection").performClick()
        compose.waitUntil { repository.sync.value.config.provider == SyncProviderModel.WebDav }
        assertTrue(
            repository.sync.value.config.webdavUrl.contains("dav.example.com")
        )

        compose.onNodeWithTag("sync-auto").performScrollTo().performClick()
        compose.waitUntil { repository.sync.value.config.autoSyncEnabled }
        assertTrue(repository.sync.value.config.autoSyncEnabled)
    }

    @Test
    fun conflictDialogKeepsRemoteOnlyAfterAnExplicitChoice() {
        repository.seedSync(
            SyncStateModel(
                phase = SyncPhaseModel.Conflict,
                config = repository.sync.value.config.copy(
                    provider = SyncProviderModel.WebDav,
                    autoSyncEnabled = true
                ),
                pendingConflict = SyncConflictModel(
                    remoteAtMillis = 1_700_000_000_000L,
                    reason = "BOTH_SIDES_CHANGED"
                )
            )
        )
        setContent()
        openSyncSettings()

        compose.onNodeWithText("本地与远端均已修改", substring = true).assertIsDisplayed()
        compose.onNodeWithText("保留远端（覆盖本地）").performScrollTo().performClick()
        compose.onNodeWithText("覆盖本地配置？").assertIsDisplayed()
        assertTrue(repository.sync.value.pendingConflict != null)
        compose.onNodeWithText("取消").performClick()
        assertTrue(repository.sync.value.pendingConflict != null)
        compose.onNodeWithText("保留远端（覆盖本地）").performClick()
        compose.onNodeWithText("确认").performClick()
        compose.waitForIdle()

        assertEquals(null, repository.sync.value.pendingConflict)
        assertTrue(repository.sync.value.phase == SyncPhaseModel.Idle)
    }

    @Test
    fun syncStatusRowsShowPhaseAndCapability() {
        repository.seedSync(
            SyncStateModel(
                phase = SyncPhaseModel.PausedVaultLocked,
                config = repository.sync.value.config.copy(
                    provider = SyncProviderModel.Gist,
                    hasGithubToken = true,
                    hasPassphrase = true,
                    capabilityProbed = true
                ),
                capability = dev.akko.miaominal.data.SyncCapabilityModel(state = "UNCHECKED"),
                vaultLocked = true
            )
        )
        setContent()
        openSyncSettings()

        compose.onNodeWithText("已暂停（保险库锁定）").assertIsDisplayed()
        compose.onNodeWithText("保险库已锁定：自动同步已暂停，解锁后可继续").assertIsDisplayed()
        compose.onNodeWithTag("sync-advanced").performScrollTo().performClick()
        compose.onNodeWithText("未检查").performScrollTo().assertIsDisplayed()
    }

    @Test fun connectionDraftCanBeKeptOrDiscardedWithoutWritingToTheBackend() {
        repository.seedSync(SyncStateModel(phase = SyncPhaseModel.Idle, config = repository.sync.value.config.copy(
            provider = SyncProviderModel.Gist, gistId = "abcdef", hasGithubToken = true, hasPassphrase = true)))
        val saved = repository.sync.value.config
        setContent()
        openSyncSettings()
        compose.onNodeWithTag("sync-edit-connection").performScrollTo().performClick()
        compose.onNodeWithTag("sync-gist-id").performScrollTo().performTextReplacement("abcdef12")
        assertEquals(saved, repository.sync.value.config)
        compose.onNodeWithTag("sync-editor-back").performClick()
        compose.onNodeWithText("放弃本次修改？").assertIsDisplayed()
        compose.onNodeWithText("取消").performClick()
        compose.onNodeWithTag("sync-gist-id").assertTextContains("abcdef12")
        compose.onNodeWithTag("sync-editor-back").performClick()
        compose.onNodeWithText("确认").performClick()
        compose.onNodeWithTag("sync-connection-editor").assertDoesNotExist()
        assertEquals(saved, repository.sync.value.config)
    }

    @Test fun failedConnectionSaveKeepsTheDraftAndShowsTheFailure() {
        repository.seedSync(SyncStateModel(phase = SyncPhaseModel.Idle, config = repository.sync.value.config.copy(
            provider = SyncProviderModel.Gist, gistId = "abcdef", hasGithubToken = true)))
        injectedRepository = object : MiaominalRepository by repository {
            override suspend fun updateSyncConfig(edit: SyncConfigEdit): Result<Unit> =
                Result.failure(IllegalStateException("测试：保存失败"))
        }
        setContent()
        openSyncSettings()
        compose.onNodeWithTag("sync-edit-connection").performScrollTo().performClick()
        compose.onNodeWithTag("sync-gist-id").performScrollTo().performTextReplacement("abcdef12")
        compose.onNodeWithTag("sync-save-connection").performClick()
        compose.waitUntil { compose.onAllNodesWithText("测试：保存失败").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText("测试：保存失败").assertIsDisplayed()
        compose.onNodeWithTag("sync-gist-id").performScrollTo().assertTextContains("abcdef12")
        assertEquals("abcdef", repository.sync.value.config.gistId)
    }

    @Test fun remoteDownloadOnlyRunsAfterAnExplicitOverwriteConfirmation() {
        repository.seedSync(SyncStateModel(phase = SyncPhaseModel.Disabled, config = repository.sync.value.config.copy(
            provider = SyncProviderModel.Gist, gistId = "abcdef", hasGithubToken = true, hasPassphrase = true)))
        injectedRepository = object : MiaominalRepository by repository {
            override suspend fun syncPull(force: Boolean): Result<Unit> {
                val result = repository.syncPull(force)
                // Match the native backend's resting phase with automatic sync off.
                repository.seedSync(repository.sync.value.copy(phase = SyncPhaseModel.Disabled))
                return result
            }
        }
        setContent()
        openSyncSettings()
        compose.onNodeWithTag("sync-overview-title").assertTextContains("配置已就绪")
        compose.onNodeWithTag("sync-auto-summary").assertTextContains("自动同步已关闭")
        compose.onNodeWithTag("sync-download").assertIsEnabled()
        compose.onNodeWithTag("sync-download").performClick()
        compose.onNodeWithText("用远端配置替换此设备？").assertIsDisplayed()
        assertEquals(SyncStatusModel.Idle, repository.sync.value.status)
        compose.onNodeWithText("取消").performClick()
        assertEquals(SyncStatusModel.Idle, repository.sync.value.status)
        compose.onNodeWithTag("sync-download").performClick()
        compose.onNodeWithText("确认").performClick()
        compose.waitUntil { repository.sync.value.status is SyncStatusModel.Pulled }
        compose.onNodeWithTag("sync-overview-title").assertTextContains("已从远端下载配置")
    }
}
