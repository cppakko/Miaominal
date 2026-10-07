package dev.akko.miaominal

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.akko.miaominal.data.AuthMethod
import dev.akko.miaominal.data.CredentialHealth
import dev.akko.miaominal.data.CredentialUpdate
import dev.akko.miaominal.data.FirstSwitchMigration
import dev.akko.miaominal.data.MiaominalRepository
import dev.akko.miaominal.data.NativeBackendClient
import dev.akko.miaominal.data.SessionProfile
import dev.akko.miaominal.data.VaultMode
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * Data and vault acceptance on a device: business data and credentials survive a backend
 * handle restart, and the local vault locks/unlocks and switches correctly.
 *
 * `Backend::restart` replaces the native handle over the same business root,
 * which is the closest deterministic simulation of a fresh process available
 * to instrumentation tests.
 */
@RunWith(AndroidJUnit4::class)
class DataAndVaultTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val client = NativeBackendClient.get(context)

    private val application: MiaominalApplication
        get() = context.applicationContext as MiaominalApplication
    private val repository: MiaominalRepository
        get() = application.container.repository

    private val dataDir: File
        get() = File(context.noBackupFilesDir, "backend-v2")

    @Before
    fun resetLocalData() = runBlocking {
        withTimeout(30_000) { repository.ensureStarted() }.getOrThrow()
        repository.resetLocalData().getOrThrow()
        repository.refreshAll().getOrThrow()
    }

    private fun sampleHost(name: String) = SessionProfile(
        id = "",
        name = name,
        group = "P2",
        host = "p2.example.com",
        port = 2222,
        username = "akko",
        authMethod = AuthMethod.Password
    )

    private fun accounts(): List<String> = application.container.credentialStore.accounts()

    @Test
    fun vaultStartsLockedAfterABackendRestart() = runBlocking<Unit> {
        val saved = repository.saveHost(
            sampleHost("locked-restart"),
            CredentialUpdate.Set("restart-secret")
        ).getOrThrow()
        repository.vaultEnable("restart-pass").getOrThrow()
        assertEquals(VaultMode.Unlocked, repository.vault.value.mode)

        // A fresh native handle re-reads the persisted vault flag: the data
        // layer must come up locked, never on the platform backend.
        client.restart()
        withTimeout(30_000) { repository.ensureStarted() }.getOrThrow()
        repository.refreshAll().getOrThrow()

        assertEquals(VaultMode.Locked, repository.vault.value.mode)
        assertEquals(CredentialHealth.Locked, repository.hostPasswordHealth(saved.id))
        val rejected = repository.saveHost(
            saved.copy(name = "locked-restart"),
            CredentialUpdate.Set("must-not-leak")
        )
        assertTrue(rejected.isFailure)

        repository.vaultUnlock("restart-pass").getOrThrow()
        assertEquals(VaultMode.Unlocked, repository.vault.value.mode)
        assertEquals(CredentialHealth.Present, repository.hostPasswordHealth(saved.id))
    }

    @Test
    fun corruptCredentialFileIsReportedAndNeverOverwritten() = runBlocking<Unit> {
        repository.saveHost(
            sampleHost("corrupt-a"),
            CredentialUpdate.Set("first-secret")
        ).getOrThrow()
        val first = repository.saveHost(
            sampleHost("corrupt-b"),
            CredentialUpdate.Set("second-secret")
        ).getOrThrow()
        val file = File(
            File(context.noBackupFilesDir, "native-credentials"),
            "platform_credentials.json"
        )
        assertTrue(file.exists())
        file.writeText("not a credential document", Charsets.UTF_8)

        assertEquals(CredentialHealth.Corrupt, repository.hostPasswordHealth(first.id))
        val before = file.readText(Charsets.UTF_8)
        val rejected = repository.saveHost(
            sampleHost("corrupt-c"),
            CredentialUpdate.Set("third-secret")
        )
        assertTrue(rejected.isFailure)
        assertEquals(before, file.readText(Charsets.UTF_8))
        file.delete()
    }

    @Test
    fun emptyCredentialFileIsReportedAndNeverOverwritten() = runBlocking<Unit> {
        val saved = repository.saveHost(
            sampleHost("empty-a"),
            CredentialUpdate.Set("first-secret")
        ).getOrThrow()
        val file = File(
            File(context.noBackupFilesDir, "native-credentials"),
            "platform_credentials.json"
        )
        assertTrue(file.exists())
        // A real empty store is `{}`; a zero-byte file is truncation damage.
        file.writeText("", Charsets.UTF_8)

        assertEquals(CredentialHealth.Corrupt, repository.hostPasswordHealth(saved.id))
        val rejected = repository.saveHost(
            sampleHost("empty-b"),
            CredentialUpdate.Set("second-secret")
        )
        assertTrue(rejected.isFailure)
        assertEquals("", file.readText(Charsets.UTF_8))
        file.delete()
    }

    @Test
    fun hostsAndCredentialsSurviveABackendRestart() = runBlocking {
        val saved = repository.saveHost(
            sampleHost("restart-host"),
            CredentialUpdate.Set("p2-secret")
        ).getOrThrow()
        assertTrue(accounts().contains("${saved.id}:password"))

        // A fresh native handle drops every in-memory cache and reopens the
        // same business root: the new-process semantics for backend state.
        client.restart()
        withTimeout(30_000) { repository.ensureStarted() }.getOrThrow()
        repository.refreshAll().getOrThrow()

        val reloaded = repository.hosts.value.firstOrNull { it.id == saved.id }
        assertEquals("restart-host", reloaded?.name)
        assertEquals(CredentialHealth.Present, repository.hostPasswordHealth(saved.id))
    }

    @Test
    fun vaultLockBlocksCredentialReadsAndUnlockRestoresThem() = runBlocking {
        val saved = repository.saveHost(
            sampleHost("vault-host"),
            CredentialUpdate.Set("vault-secret")
        ).getOrThrow()

        repository.vaultEnable("correct horse").getOrThrow()
        assertEquals(VaultMode.Unlocked, repository.vault.value.mode)
        assertTrue(File(dataDir, "secret_vault.json").exists())
        // The platform copy is cleaned up after the switch.
        assertTrue(!accounts().contains("${saved.id}:password"))

        repository.vaultLock().getOrThrow()
        assertEquals(VaultMode.Locked, repository.vault.value.mode)
        assertEquals(CredentialHealth.Locked, repository.hostPasswordHealth(saved.id))

        val wrong = repository.vaultUnlock("wrong horse")
        assertTrue(wrong.isFailure)
        assertEquals(VaultMode.Locked, repository.vault.value.mode)

        repository.vaultUnlock("correct horse").getOrThrow()
        assertEquals(VaultMode.Unlocked, repository.vault.value.mode)
        assertEquals(CredentialHealth.Present, repository.hostPasswordHealth(saved.id))

        // Round trip back to the platform backend restores the Keystore copy.
        repository.vaultDisable().getOrThrow()
        assertEquals(VaultMode.Disabled, repository.vault.value.mode)
        assertTrue(accounts().contains("${saved.id}:password"))
        assertEquals(CredentialHealth.Present, repository.hostPasswordHealth(saved.id))
    }

    @Test
    fun switchingToAVaultKeepsWorkingCredentialsReadable() = runBlocking {
        val first = repository.saveHost(
            sampleHost("switch-first"),
            CredentialUpdate.Set("first-secret")
        ).getOrThrow()
        val second = repository.saveHost(
            sampleHost("switch-second"),
            CredentialUpdate.Set("second-secret")
        ).getOrThrow()

        repository.vaultEnable("switch-pass").getOrThrow()
        assertEquals(CredentialHealth.Present, repository.hostPasswordHealth(first.id))
        assertEquals(CredentialHealth.Present, repository.hostPasswordHealth(second.id))

        repository.vaultLock().getOrThrow()
        repository.vaultUnlock("switch-pass").getOrThrow()
        assertEquals(CredentialHealth.Present, repository.hostPasswordHealth(first.id))
    }

    @Test
    fun failedVaultEnableKeepsThePlatformCredentials() = runBlocking<Unit> {
        val saved = repository.saveHost(
            sampleHost("failing-host"),
            CredentialUpdate.Set("platform-secret")
        ).getOrThrow()
        // A corrupt vault file makes the enable flow fail before the switch.
        File(dataDir, "secret_vault.json").writeText("not a vault document")

        val result = repository.vaultEnable("some passphrase")

        assertTrue(result.isFailure)
        assertEquals(VaultMode.Disabled, repository.vault.value.mode)
        assertEquals(CredentialHealth.Present, repository.hostPasswordHealth(saved.id))
        assertTrue(accounts().contains("${saved.id}:password"))
        File(dataDir, "secret_vault.json").delete()
        Unit
    }

    @Test
    fun aFreshBusinessRootShowsTheFirstLaunchOnboarding() = runBlocking<Unit> {
        // Rebuild the root the way a brand-new install starts (no settings or
        // store files at all). Installation-level markers live outside the
        // Rust root; anything still inside is real app data, and the settings
        // store would then auto-complete onboarding and hide the init screen.
        client.shutdown()
        dataDir.listFiles().orEmpty().forEach { file ->
            if (file.name != ".p2-initialized") file.deleteRecursively()
        }
        withTimeout(30_000) { client.ensureStarted() }
        repository.refreshAll().getOrThrow()

        assertFalse(repository.settings.value.onboardingCompleted)
    }

    @Test
    fun aLegacyMarkerInsideTheBusinessRootIsPromoted() = runBlocking<Unit> {
        val marker = File(context.noBackupFilesDir, ".p2-initialized")
        val legacyMarker = File(dataDir, ".p2-initialized")
        marker.delete()
        legacyMarker.writeText("miaominal-android/p2\n", Charsets.UTF_8)

        FirstSwitchMigration.runIfNeeded(context)

        assertTrue(marker.exists())
        assertFalse(legacyMarker.exists())
    }
}
