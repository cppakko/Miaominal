package dev.akko.miaominal.ui.model

import dev.akko.miaominal.data.*
import dev.akko.miaominal.R
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class SyncSetupTest {
    @Test fun configuredManualSyncIsReadyEvenWhenTheAutomaticWorkerIsDisabled() {
        val state = manualSyncState()
        assertEquals(R.string.sync_mobile_ready, syncOverviewTitle(state))
        assertEquals(R.string.sync_mobile_ready, syncOverviewTitle(state.copy(
            config = state.config.copy(provider = SyncProviderModel.WebDav,
                webdavUrl = "https://dav.example.com/config", hasWebdavPassword = true))))
        assertEquals(R.string.sync_mobile_ready, syncOverviewTitle(state.copy(
            config = state.config.copy(gistId = "")))) // First upload creates a Gist.
    }

    @Test fun manualCompletionRemainsVisibleWithAutomaticSyncOff() {
        val state = manualSyncState()
        listOf(SyncPhaseModel.Idle, SyncPhaseModel.Disabled).forEach { phase ->
            listOf(
                SyncStatusModel.Pushed(1) to R.string.sync_mobile_uploaded,
                SyncStatusModel.Pulled(1) to R.string.sync_mobile_downloaded,
                SyncStatusModel.UpToDate(1) to R.string.sync_mobile_up_to_date,
                SyncStatusModel.Cancelled to R.string.ui_sync_cancelled
            ).forEach { (status, title) ->
                assertEquals(title, syncOverviewTitle(state.copy(phase = phase, status = status)))
            }
        }
    }

    @Test fun loadingSetupErrorsConflictsAndPausedWorkKeepTheirMeaning() {
        val state = manualSyncState()
        assertEquals(R.string.sync_mobile_loading, syncOverviewTitle(state.copy(loaded = false)))
        listOf(state.config.copy(provider = SyncProviderModel.None),
            state.config.copy(hasGithubToken = false), state.config.copy(hasPassphrase = false),
            state.config.copy(provider = SyncProviderModel.WebDav, hasWebdavPassword = true,
                webdavUrl = "invalid")).forEach { config ->
            assertEquals(R.string.sync_mobile_setup_title, syncOverviewTitle(state.copy(config = config)))
        }
        val failure = SyncFailureModel("NETWORK", "offline")
        assertEquals(R.string.sync_mobile_last_failed, syncOverviewTitle(state.copy(lastFailure = failure)))
        assertEquals(R.string.sync_mobile_last_failed, syncOverviewTitle(state.copy(status = SyncStatusModel.Failed(failure))))
        assertEquals(R.string.sync_mobile_conflict_title, syncOverviewTitle(state.copy(
            pendingConflict = SyncConflictModel(1, "BOTH_SIDES_CHANGED"))))
        assertEquals(R.string.ui_paused_vault_locked, syncOverviewTitle(state.copy(vaultLocked = true)))
        listOf(SyncPhaseModel.Pushing, SyncPhaseModel.Pulling, SyncPhaseModel.CheckingRemote,
            SyncPhaseModel.Cancelling, SyncPhaseModel.PausedCredentials,
            SyncPhaseModel.PausedCapability).forEach { phase ->
            assertNull(syncOverviewTitle(state.copy(phase = phase)))
        }
    }

    private fun manualSyncState() = SyncStateModel(loaded = true, phase = SyncPhaseModel.Disabled,
        config = SyncConfigModel(provider = SyncProviderModel.Gist, gistId = "abcdef",
            hasGithubToken = true, hasPassphrase = true, autoSyncEnabled = false))

    @Test fun validationAcceptsPastedGistLinksAndMatchesBackendUrlRules() {
        assertEquals("abcdef1234", normalizeGistId(" https://gist.github.com/user/abcdef1234#file-config "))
        assertEquals("", normalizeGistId(" "))
        assertNull(normalizeGistId("https://github.com/user/abcdef1234"))
        assertNull(normalizeGistId("not a gist"))
        assertTrue(validWebDavUrl("https://dav.example.com/config.json"))
        assertTrue(validWebDavUrl("http://127.0.0.1:8080/config"))
        assertTrue(validWebDavUrl("http://[::1]:8080/config"))
        assertFalse(validWebDavUrl("http://dav.example.com/config"))
        assertFalse(validWebDavUrl("https://user:password@dav.example.com/config"))
        assertFalse(validWebDavUrl("https://dav.example.com/config#fragment"))
        val draft = SyncConnectionDraft(SyncProviderModel.WebDav, webdavUrl = "bad")
        assertEquals(SyncSetupIssue.WebDavUrl, draft.firstIssue(SyncConfigModel()))
        assertEquals(SyncSetupIssue.Credential, draft.copy(webdavUrl = "https://dav.example.com/config").firstIssue(SyncConfigModel()))
    }

    @Test fun changingDestinationSavesCredentialFirstAndRevokesAutomaticUnsafeWrites() = runBlocking {
        val base = configuredRepository()
        val calls = mutableListOf<String>()
        val repository = object : MiaominalRepository by base {
            override suspend fun syncSetAutoSync(enabled: Boolean): Result<Unit> {
                calls += "automatic:$enabled"
                return base.syncSetAutoSync(enabled)
            }
            override suspend fun setSyncCredential(kind: SyncCredentialKindModel, update: CredentialUpdate): Result<Unit> {
                calls += "credential"
                assertEquals(" password with spaces ", (update as CredentialUpdate.Set).value)
                return base.setSyncCredential(kind, update)
            }
            override suspend fun updateSyncConfig(edit: SyncConfigEdit): Result<Unit> {
                calls += "configuration"
                return base.updateSyncConfig(edit)
            }
        }
        saveSyncConnection(repository, SyncConnectionDraft(SyncProviderModel.WebDav,
            webdavUrl = " https://dav.example.com/config ", webdavUsername = " user ",
            credential = " password with spaces ")).getOrThrow()
        assertEquals(listOf("automatic:false", "credential", "configuration"), calls)
        val config = base.sync.value.config
        assertEquals(SyncProviderModel.WebDav, config.provider)
        assertEquals("https://dav.example.com/config", config.webdavUrl)
        assertEquals("user", config.webdavUsername)
        assertEquals("abcdef", config.gistId) // Inactive provider's setup is retained.
        assertTrue(config.hasWebdavPassword)
        assertFalse(config.autoSyncEnabled)
        assertFalse(config.webdavUnsafeWriteConsent)
    }

    @Test fun blankCredentialKeepsStoredSecretAndAnUnchangedDestinationKeepsAutoSync() = runBlocking {
        val base = configuredRepository()
        val repository = object : MiaominalRepository by base {
            override suspend fun setSyncCredential(kind: SyncCredentialKindModel, update: CredentialUpdate): Result<Unit> =
                error("Blank must not delete or replace an existing credential")
        }
        saveSyncConnection(repository, SyncConnectionDraft(SyncProviderModel.Gist, gistId = "abcdef")).getOrThrow()
        assertTrue(base.sync.value.config.hasGithubToken)
        assertTrue(base.sync.value.config.autoSyncEnabled)
    }

    @Test fun invalidDraftWritesNothingAndCredentialFailureNeverActivatesNewDestination() = runBlocking {
        val base = configuredRepository()
        val before = base.sync.value
        assertTrue(saveSyncConnection(base, SyncConnectionDraft(SyncProviderModel.WebDav, webdavUrl = "bad")).isFailure)
        assertEquals(before, base.sync.value)
        val repository = object : MiaominalRepository by base {
            override suspend fun setSyncCredential(kind: SyncCredentialKindModel, update: CredentialUpdate): Result<Unit> =
                Result.failure(IllegalStateException("Credential storage failed"))
        }
        val failure = saveSyncConnection(repository, SyncConnectionDraft(SyncProviderModel.WebDav,
            webdavUrl = "https://dav.example.com/config", credential = "password")).exceptionOrNull() as SyncSetupSaveException
        assertEquals(SyncSaveStage.Credential, failure.stage)
        assertFalse(failure.credentialSaved)
        assertEquals(SyncProviderModel.Gist, base.sync.value.config.provider)
        assertFalse(base.sync.value.config.autoSyncEnabled)
    }

    @Test fun partialSaveIsExplicitAndRemovingCredentialRequiresTheDraftChoice() = runBlocking {
        val base = configuredRepository()
        val repository = object : MiaominalRepository by base {
            override suspend fun updateSyncConfig(edit: SyncConfigEdit): Result<Unit> =
                Result.failure(IllegalStateException("Configuration storage failed"))
        }
        val failure = saveSyncConnection(repository, SyncConnectionDraft(SyncProviderModel.WebDav,
            webdavUrl = "https://dav.example.com/config", credential = "password")).exceptionOrNull() as SyncSetupSaveException
        assertEquals(SyncSaveStage.Configuration, failure.stage)
        assertTrue(failure.credentialSaved)
        assertTrue(base.sync.value.config.hasWebdavPassword)
        assertEquals(SyncProviderModel.Gist, base.sync.value.config.provider)
        saveSyncConnection(base, SyncConnectionDraft(SyncProviderModel.Gist, gistId = "abcdef", removeCredential = true)).getOrThrow()
        assertFalse(base.sync.value.config.hasGithubToken)
        assertFalse(base.sync.value.config.autoSyncEnabled)
    }

    @Test fun passphraseConfirmationPreservesTheExactSecret() {
        assertEquals(SyncPassphraseIssue.Empty, syncPassphraseIssue(" ", " "))
        assertEquals(SyncPassphraseIssue.Mismatch, syncPassphraseIssue(" secret ", "secret"))
        assertNull(syncPassphraseIssue(" secret ", " secret "))
    }

    private fun configuredRepository() = InMemoryMiaominalRepository().apply {
        seedSync(SyncStateModel(phase = SyncPhaseModel.Idle, config = SyncConfigModel(
            provider = SyncProviderModel.Gist, gistId = "abcdef", hasGithubToken = true,
            hasPassphrase = true, autoSyncEnabled = true, webdavUnsafeWriteConsent = true)))
    }
}
