package dev.akko.miaominal

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.CredentialHealth
import dev.akko.miaominal.data.CredentialUpdate
import dev.akko.miaominal.data.ImportSourceModel
import dev.akko.miaominal.data.MiaominalRepository
import dev.akko.miaominal.data.SyncConfigEdit
import dev.akko.miaominal.data.SyncCredentialKindModel
import dev.akko.miaominal.data.SyncProviderModel
import dev.akko.miaominal.data.VaultMode
import dev.akko.miaominal.nativebridge.BackendException
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * acceptance against the real native backend: typed sync configuration and
 * credentials, the auto-sync capability gate, vault-locked rejection, legacy
 * credential migration, and content-based configuration import.
 *
 * No test here touches the public network: providers are configured with
 * unreachable local endpoints on purpose, so failures stay typed and bounded.
 */
@RunWith(AndroidJUnit4::class)
class NativeSyncTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private val application: MiaominalApplication
        get() = context.applicationContext as MiaominalApplication
    private val repository: MiaominalRepository
        get() = application.container.repository

    @Before
    fun resetLocalData() = runBlocking {
        withTimeout(30_000) { repository.ensureStarted() }.getOrThrow()
        repository.resetLocalData().getOrThrow()
        repository.refreshAll().getOrThrow()
        repository.refreshSync()
    }

    @Test
    fun syncConfigAndCredentialsUseTheBackendStore() = runBlocking<Unit> {
        repository.updateSyncConfig(
            SyncConfigEdit(
                provider = SyncProviderModel.Gist,
                gistId = "gist-123",
                webdavUnsafeWriteConsent = false
            )
        ).getOrThrow()
        repository.setSyncCredential(
            SyncCredentialKindModel.GithubToken,
            CredentialUpdate.Set("token-value")
        ).getOrThrow()

        var config = repository.sync.value.config
        assertEquals(SyncProviderModel.Gist, config.provider)
        assertEquals("gist-123", config.gistId)
        assertTrue(config.hasGithubToken)
        assertEquals(
            CredentialHealth.Present,
            repository.syncCredentialHealth(SyncCredentialKindModel.GithubToken)
        )

        repository.setSyncCredential(
            SyncCredentialKindModel.GithubToken,
            CredentialUpdate.Clear
        ).getOrThrow()
        config = repository.sync.value.config
        assertTrue(!config.hasGithubToken)
        assertEquals(
            CredentialHealth.Missing,
            repository.syncCredentialHealth(SyncCredentialKindModel.GithubToken)
        )

        // Switching providers keeps the other provider's stored fields (the
        // desktop behaviour); only the remote binding baseline is reset.
        repository.updateSyncConfig(SyncConfigEdit(provider = SyncProviderModel.WebDav)).getOrThrow()
        config = repository.sync.value.config
        assertEquals(SyncProviderModel.WebDav, config.provider)
        assertEquals("gist-123", config.gistId)
    }

    @Test
    fun autoSyncCannotEnableWithoutAReachableCapabilityProbe() = runBlocking<Unit> {
        repository.updateSyncConfig(
            SyncConfigEdit(
                provider = SyncProviderModel.WebDav,
                webdavUrl = "http://127.0.0.1:1/sync.json",
                webdavUsername = "user"
            )
        ).getOrThrow()
        repository.setSyncCredential(
            SyncCredentialKindModel.WebdavPassword,
            CredentialUpdate.Set("password")
        ).getOrThrow()
        repository.setSyncCredential(
            SyncCredentialKindModel.Passphrase,
            CredentialUpdate.Set("passphrase")
        ).getOrThrow()

        val result = repository.syncSetAutoSync(true)
        assertTrue("an unreachable remote must not enable auto sync", result.isFailure)
        val failure = result.exceptionOrNull()
        assertTrue(
            "expected a typed capability/network failure, got $failure",
            failure is BackendException.SyncCapability || failure is BackendException.Network
        )
        assertTrue(!repository.sync.value.config.autoSyncEnabled)
    }

    @Test
    fun aLockedVaultRejectsSyncUntilUnlocked() = runBlocking<Unit> {
        repository.updateSyncConfig(
            SyncConfigEdit(
                provider = SyncProviderModel.WebDav,
                webdavUrl = "http://127.0.0.1:1/sync.json",
                webdavUsername = "user"
            )
        ).getOrThrow()
        repository.setSyncCredential(
            SyncCredentialKindModel.WebdavPassword,
            CredentialUpdate.Set("password")
        ).getOrThrow()
        repository.setSyncCredential(
            SyncCredentialKindModel.Passphrase,
            CredentialUpdate.Set("passphrase")
        ).getOrThrow()

        repository.vaultEnable("p6-vault-pass").getOrThrow()
        repository.vaultLock().getOrThrow()
        assertEquals(VaultMode.Locked, repository.vault.value.mode)

        val locked = repository.syncPush(manual = true)
        assertTrue(locked.isFailure)
        assertTrue(
            "a locked vault must fail with VaultLocked, got ${locked.exceptionOrNull()}",
            locked.exceptionOrNull() is BackendException.VaultLocked
        )

        repository.vaultUnlock("p6-vault-pass").getOrThrow()
        val unlocked = repository.syncPush(manual = true)
        assertTrue(
            "after unlock the failure must not be VaultLocked",
            unlocked.exceptionOrNull() !is BackendException.VaultLocked
        )
    }

    @Test
    fun legacyDeviceEntriesMigrateAndAreRemoved() = runBlocking<Unit> {
        val credentialStore = application.container.credentialStore
        credentialStore.set("sync:gist-token", "legacy-token")
        credentialStore.set("sync:sync-passphrase", "legacy-passphrase")

        repository.syncMigrateLegacyCredentials().getOrThrow()

        assertNull(credentialStore.get("sync:gist-token"))
        assertNull(credentialStore.get("sync:sync-passphrase"))
        assertEquals(
            CredentialHealth.Present,
            repository.syncCredentialHealth(SyncCredentialKindModel.GithubToken)
        )
        assertEquals(
            CredentialHealth.Present,
            repository.syncCredentialHealth(SyncCredentialKindModel.Passphrase)
        )
        assertTrue(repository.sync.value.config.hasGithubToken)
        assertTrue(repository.sync.value.config.hasPassphrase)
    }

    @Test
    fun importPreviewsWarningsAndCommitsOnlyAfterConfirmation() = runBlocking<Unit> {
        val content = buildString {
            appendLine("Host p6-imported")
            appendLine("  HostName imported.example.com")
            appendLine("  User akko")
            appendLine("  ProxyJump bastion")
        }
        val preview = repository.previewImport(
            ImportSourceModel.OpenSsh,
            "config",
            content.toByteArray()
        ).getOrThrow()

        assertEquals(1, preview.entries.size)
        assertEquals("imported.example.com", preview.entries[0].host)
        assertTrue(
            "ProxyJump must be reported as a warning",
            preview.warnings.any { it.reason.contains("proxy_jump") }
        )
        // Nothing is committed before the user confirms.
        assertTrue(repository.hosts.value.none { it.name == "p6-imported" })

        val imported = repository.commitImport(preview.previewId).getOrThrow()
        assertEquals(1, imported)
        assertTrue(repository.hosts.value.any { it.name == "p6-imported" })

        // The preview is consumed: a second commit must fail.
        assertTrue(repository.commitImport(preview.previewId).isFailure)
    }

    @Test
    fun importRejectsOversizedDocuments() = runBlocking<Unit> {
        val oversized = ByteArray(9 * 1024 * 1024) { 'a'.code.toByte() }
        val result = repository.previewImport(
            ImportSourceModel.OpenSsh,
            "huge-config",
            oversized
        )
        assertTrue("oversized files must be refused", result.isFailure)
        assertTrue(result.exceptionOrNull() is BackendException.InvalidArgument)
    }

    @Test
    fun syncSnapshotReportsAConfiguredProvider() = runBlocking<Unit> {
        repository.updateSyncConfig(
            SyncConfigEdit(
                provider = SyncProviderModel.WebDav,
                webdavUrl = "http://127.0.0.1:1/sync.json",
                webdavUsername = "user"
            )
        ).getOrThrow()
        repository.refreshSync()
        val state = repository.sync.value
        assertEquals(SyncProviderModel.WebDav, state.config.provider)
        assertNotNull(state.config.deviceId)
        assertTrue(state.config.deviceId.isNotBlank())
        // A configured-but-unchecked endpoint stays idle and does not claim
        // capability support.
        assertTrue(state.capability.state != "SUPPORTED")
    }
}
